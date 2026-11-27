#!/usr/bin/env python3
"""Rebuild the `bmu` reference table: units, names, fuel types and locations.

The table was loaded once (migration 0004) from a vendored 2022-era CSV. This
tool re-derives it from live sources and writes a SQL migration that upserts
the result, plus a review CSV of every location it assigned.

Units and names come from the Elexon reference list (/reference/bmunits/all).

Fuel type (`bmrs_fuel_type`, the BMRS vocabulary the dashboards group by), in
order of preference:
  1. Elexon's fuelType.
  2. The existing value.
  3. A value inferred from the matched site's technology (battery and solar
     become OTHER, as BMRS classes them). `reg_fuel_type` keeps the specific
     technology (BATTERY, SOLAR, ...).

Location, in order of preference (the first that applies wins):
  1. The original 0004 coordinates. These are never touched.
  2. Wikidata: an item with P11610 (National Grid BM unit ID) = this unit,
     taking its P625 coordinates or its parent station's.
  3. REPD, via the REPD ID from Wikidata (P9891) or from the existing table.
  4. Station-demand units (CLACD-1, GORDD-1) take their generator's location
     (CLAC-1, GORDW-1).
  5. An exact match of the station name, plus technology and capacity
     checks, against REPD (operational, under construction or awaiting
     construction) or DUKES 5.11 thermal/nuclear. DUKES wind and hydro
     coordinates fail validation and are not used. Single-site units only
     (T_ / E_ / M_). Supplier (2__), virtual (V__) and interconnector (I_)
     units have no single location and are left NULL.
Locations a previous refresh assigned are recomputed every time, so running a
newer refresh corrects them. `--validate` name-matches every unit whose
location is already known and reports the error by source and technology.
Every coordinate must fall inside a GB-and-waters bounding box.

Usage (needs network; openpyxl for the DUKES xlsx):
    uv run --no-project --with openpyxl==3.1.5 python tools/bmu/refresh_bmu.py \
        --cache /tmp/bmu-cache --out src/gb_grid/migrations/0015.bmu_refresh.sql
"""

from __future__ import annotations

import argparse
import csv
import difflib
import io
import json
import math
import re
import sys
import urllib.parse
import urllib.request
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BASE_CSV = ROOT / "src/gb_grid/migrations/0004_bmu.csv"
UA = "gb-grid-bmu-refresh/0.1 (data-prep script; https://github.com/chrsphr/gb-grid)"
ELEXON = "https://data.elexon.co.uk/bmrs/api/v1/reference/bmunits/all"
REPD_PAGE = "https://www.gov.uk/api/content/government/publications/renewable-energy-planning-database-quarterly-extract"
DUKES_PAGE = "https://www.gov.uk/api/content/government/statistics/electricity-chapter-5-digest-of-united-kingdom-energy-statistics-dukes"
WIKIDATA = "https://query.wikidata.org/sparql"
SPARQL = """
SELECT ?item ?itemLabel ?bmu ?coord ?parentCoord ?repd ?typeLabel WHERE {
  ?item wdt:P11610 ?bmu .
  OPTIONAL { ?item wdt:P625 ?coord }
  OPTIONAL { ?item wdt:P361 ?parent . ?parent wdt:P625 ?parentCoord }
  OPTIONAL { ?item wdt:P9891 ?repd }
  OPTIONAL { ?item wdt:P31 ?type }
  SERVICE wikibase:label { bd:serviceParam wikibase:language "en". }
}"""

# GB, Northern Ireland and offshore wind zones (Dogger Bank reaches ~2.6E).
BBOX = (49.0, 61.5, -9.0, 4.0)  # lat min/max, lon min/max


# --------------------------------------------------------------------------- io


def fetch(url: str, cache: Path, name: str, *, headers: dict | None = None) -> bytes:
    path = cache / name
    if path.exists():
        return path.read_bytes()
    req = urllib.request.Request(url, headers={"User-Agent": UA, **(headers or {})})
    with urllib.request.urlopen(req, timeout=300) as r:
        data = r.read()
    path.write_bytes(data)
    return data


def gov_attachment(page: str, cache: Path, name: str, want: str) -> bytes:
    """Download the first attachment of a gov.uk publication whose URL ends in `want`."""
    meta = json.loads(fetch(page, cache, name + ".page.json"))
    for a in meta["details"]["attachments"]:
        if a.get("url", "").lower().endswith(want):
            print(f"  {a['title']}", file=sys.stderr)
            return fetch(a["url"], cache, name)
    raise SystemExit(f"no {want} attachment on {page}")


# ------------------------------------------------------------------ geodesy


def osgb_to_wgs84(e: float, n: float) -> tuple[float, float]:
    """OSGB36 National Grid easting/northing -> WGS84 lat/lon (Helmert, ~5 m)."""
    a, b = 6377563.396, 6356256.909  # Airy 1830
    f0, lat0, lon0 = 0.9996012717, math.radians(49), math.radians(-2)
    n0, e0 = -100000, 400000
    e2 = 1 - (b * b) / (a * a)
    nn = (a - b) / (a + b)

    def meridional(lat):
        return (
            b
            * f0
            * (
                (1 + nn + 1.25 * nn**2 + 1.25 * nn**3) * (lat - lat0)
                - (3 * nn + 3 * nn**2 + 2.625 * nn**3) * math.sin(lat - lat0) * math.cos(lat + lat0)
                + (1.875 * nn**2 + 1.875 * nn**3)
                * math.sin(2 * (lat - lat0))
                * math.cos(2 * (lat + lat0))
                - (35 / 24) * nn**3 * math.sin(3 * (lat - lat0)) * math.cos(3 * (lat + lat0))
            )
        )

    lat, m = lat0, 0.0
    while True:
        lat = (n - n0 - m) / (a * f0) + lat
        m = meridional(lat)
        if abs(n - n0 - m) < 1e-5:
            break
    s, c, t = math.sin(lat), math.cos(lat), math.tan(lat)
    nu = a * f0 / math.sqrt(1 - e2 * s * s)
    rho = a * f0 * (1 - e2) / (1 - e2 * s * s) ** 1.5
    eta2 = nu / rho - 1
    vii = t / (2 * rho * nu)
    viii = t / (24 * rho * nu**3) * (5 + 3 * t * t + eta2 - 9 * t * t * eta2)
    ix = t / (720 * rho * nu**5) * (61 + 90 * t * t + 45 * t**4)
    x = 1 / (c * nu)
    xi = 1 / (c * 6 * nu**3) * (nu / rho + 2 * t * t)
    xii = 1 / (c * 120 * nu**5) * (5 + 28 * t * t + 24 * t**4)
    xiia = 1 / (c * 5040 * nu**7) * (61 + 662 * t * t + 1320 * t**4 + 720 * t**6)
    de = e - e0
    phi = lat - vii * de**2 + viii * de**4 - ix * de**6
    lam = lon0 + x * de - xi * de**3 + xii * de**5 - xiia * de**7

    # OSGB36 geodetic -> cartesian -> Helmert -> WGS84 geodetic.
    s, c = math.sin(phi), math.cos(phi)
    nu = a / math.sqrt(1 - e2 * s * s)
    x1, y1, z1 = nu * c * math.cos(lam), nu * c * math.sin(lam), (1 - e2) * nu * s
    tx, ty, tz, sc = 446.448, -125.157, 542.060, -20.4894e-6
    rx, ry, rz = (math.radians(v / 3600) for v in (0.1502, 0.2470, 0.8421))
    x2 = tx + (1 + sc) * x1 - rz * y1 + ry * z1
    y2 = ty + rz * x1 + (1 + sc) * y1 - rx * z1
    z2 = tz - ry * x1 + rx * y1 + (1 + sc) * z1
    a2, b2 = 6378137.0, 6356752.3142
    e22 = 1 - (b2 * b2) / (a2 * a2)
    p = math.hypot(x2, y2)
    phi2 = math.atan2(z2, p * (1 - e22))
    for _ in range(10):
        nu2 = a2 / math.sqrt(1 - e22 * math.sin(phi2) ** 2)
        phi2 = math.atan2(z2 + e22 * nu2 * math.sin(phi2), p)
    return round(math.degrees(phi2), 5), round(math.degrees(math.atan2(y2, x2)), 5)


def in_bbox(lat: float, lon: float) -> bool:
    return BBOX[0] <= lat <= BBOX[1] and BBOX[2] <= lon <= BBOX[3]


def km(a: tuple[float, float], b: tuple[float, float]) -> float:
    la1, lo1, la2, lo2 = map(math.radians, (*a, *b))
    h = (
        math.sin((la2 - la1) / 2) ** 2
        + math.cos(la1) * math.cos(la2) * math.sin((lo2 - lo1) / 2) ** 2
    )
    return 6371 * 2 * math.asin(math.sqrt(h))


# ------------------------------------------------------------------ sources


def load_elexon(cache: Path) -> dict[str, dict]:
    units = json.loads(fetch(ELEXON, cache, "elexon_bmunits.json"))
    return {u["nationalGridBmUnit"]: u for u in units if u.get("nationalGridBmUnit")}


def load_base() -> dict[str, dict]:
    with BASE_CSV.open(newline="", encoding="utf-8") as f:
        return {r["NESO BMU ID"].strip(): r for r in csv.DictReader(f) if r["NESO BMU ID"].strip()}


def load_wikidata(cache: Path) -> dict[str, dict]:
    url = WIKIDATA + "?" + urllib.parse.urlencode({"query": SPARQL})
    rows = json.loads(
        fetch(
            url, cache, "wikidata_bmu.json", headers={"Accept": "application/sparql-results+json"}
        )
    )["results"]["bindings"]
    v = lambda r, k: r.get(k, {}).get("value")  # noqa: E731
    out: dict[str, dict] = defaultdict(
        lambda: {"coords": set(), "repd": set(), "types": set(), "items": set()}
    )
    for r in rows:
        d = out[v(r, "bmu").strip().upper()]
        d["items"].add(v(r, "item").rsplit("/", 1)[1])
        d["label"] = v(r, "itemLabel")
        for k in ("coord", "parentCoord"):
            if v(r, k) and v(r, k).startswith("Point("):
                lon, lat = map(float, v(r, k)[6:-1].split())
                d["coords"].add((round(lat, 5), round(lon, 5)))
                break
        if v(r, "repd"):
            d["repd"].add(v(r, "repd"))
        if v(r, "typeLabel"):
            d["types"].add(v(r, "typeLabel"))
    return out


def load_repd(cache: Path) -> list[dict]:
    raw = gov_attachment(REPD_PAGE, cache, "repd.csv", ".csv")
    rows = list(csv.DictReader(io.StringIO(raw.decode("cp1252"))))
    out = []
    for r in rows:
        r = {k.strip(): (v or "").strip() for k, v in r.items() if k}
        try:
            lat, lon = osgb_to_wgs84(float(r["X-coordinate"]), float(r["Y-coordinate"]))
        except (ValueError, KeyError):
            continue
        out.append(
            {
                "src": f"repd:{r['Ref ID']}",
                "id": r["Ref ID"],
                "name": r["Site Name"],
                "tech": r["Technology Type"],
                "status": r["Development Status (short)"],
                "cap": _num(r.get("Installed Capacity (MWelec)")),
                "lat": lat,
                "lon": lon,
            }
        )
    return out


def load_dukes(cache: Path) -> list[dict]:
    import openpyxl

    raw = gov_attachment(DUKES_PAGE, cache, "dukes_5.11.xlsx", "dukes_5.11.xlsx")
    ws = openpyxl.load_workbook(io.BytesIO(raw), read_only=True, data_only=True)["5.11 Full list"]
    rows = list(ws.iter_rows(values_only=True))
    h = next(i for i, r in enumerate(rows) if r and str(r[0] or "").startswith("Company Name"))
    hdr = [str(c or "").split(" [")[0].strip() for c in rows[h]]
    out = []
    for r in rows[h + 1 :]:
        d = dict(zip(hdr, r, strict=False))  # trailing empty cells may be absent
        try:
            lat, lon = osgb_to_wgs84(float(d["X-Coordinate"]), float(d["Y-Coordinate"]))
        except (TypeError, ValueError, KeyError):
            continue
        tech = str(d.get("Technology") or "")
        if tech == "Fossil Fuel":
            tech = f"Fossil Fuel {d.get('Type') or ''}".strip()
        out.append(
            {
                "src": f"dukes:{d.get('DESNZ site code') or d['Site Name']}",
                "id": None,
                "name": str(d["Site Name"]).strip(),
                "tech": tech,
                "status": "Operational",
                "cap": _num(d.get("InstalledCapacity (MW)")),
                "lat": lat,
                "lon": lon,
            }
        )
    return out


def _num(x) -> float | None:
    try:
        return float(str(x).replace(",", ""))
    except (TypeError, ValueError):
        return None


# ------------------------------------------------------------------ matching

STATION_RE = re.compile(r"-?\d+[A-Z]?$")  # same as migration 0005
_GENERIC_WORDS = """
    wind farm wf windfarm offshore onshore bess battery batteries energy storage store
    power station ps ltd limited plc solar pv park project site the and of at bmu unit
    ut module gt st stage phase block generation generating gen plant facility scheme
    hybrid co cogen chp t e uk green demand supply repower repowering peaker peakers
    """
GENERIC = frozenset(_GENERIC_WORDS.split())

# DUKES categories whose coordinates hold up (`--validate`: fossil 20/20, nuclear
# 2/2 within 5 km). DUKES wind (46%: offshore farms sit at their onshore
# connection) and hydro (50%, e.g. Clachan placed on Skye) are never used.
DUKES_RELIABLE = frozenset(
    {
        "Fossil Fuel CCGT",
        "Fossil Fuel Single cycle",
        "Fossil Fuel Conventional steam",
        "Nuclear",
        "Bioenergy",
    }
)

TECH = {  # tag -> (REPD technologies, DUKES technologies)
    "wind": ({"Wind Onshore", "Wind Offshore"}, set()),
    "battery": ({"Battery"}, set()),
    "solar": ({"Solar Photovoltaics"}, set()),
    "hydro": ({"Small Hydro", "Large Hydro"}, set()),
    "ps": ({"Pumped Storage Hydroelectricity"}, set()),
    "biomass": (
        {
            "Biomass (dedicated)",
            "Advanced Conversion Technologies",
            "EfW Incineration",
            "Landfill Gas",
            "Anaerobic Digestion",
        },
        {"Bioenergy"},
    ),
    # DUKES labels these "Fossil Fuel " + Type (CCGT, Single cycle, Conventional steam).
    "gas": (
        set(),
        {"Fossil Fuel CCGT", "Fossil Fuel Single cycle", "Fossil Fuel Conventional steam"},
    ),
    "nuclear": (set(), {"Nuclear"}),
}
FUEL_TAG = {
    "WIND": "wind",
    "CCGT": "gas",
    "OCGT": "gas",
    "COAL": "gas",
    "NPSHYD": "hydro",
    "PS": "ps",
    "BIOMASS": "biomass",
    "NUCLEAR": "nuclear",
}
# Inferred fuel type for units Elexon leaves NULL: (bmrs_fuel_type, reg_fuel_type).
TAG_FUEL = {
    "wind": ("WIND", "WIND"),
    "battery": ("OTHER", "BATTERY"),
    "solar": ("OTHER", "SOLAR"),
    "hydro": ("NPSHYD", "NPSHYD"),
    "ps": ("PS", "PS"),
    "biomass": ("BIOMASS", "BIOMASS"),
    "nuclear": ("NUCLEAR", "NUCLEAR"),
}
WD_TAG = [
    ("battery", "battery"),
    ("wind", "wind"),
    ("solar", "solar"),
    ("photovoltaic", "solar"),
    ("pumped", "ps"),
    ("hydro", "hydro"),
    ("nuclear", "nuclear"),
    ("biomass", "biomass"),
]


def tech_tag(unit: dict, wd: dict | None) -> str | None:
    name = (unit.get("bmUnitName") or "").lower()
    # "pumped" before "storage": pumped-storage hydro is not a battery.
    for kw, tag in (
        ("pumped", "ps"),
        ("bess", "battery"),
        ("battery", "battery"),
        ("storage", "battery"),
        ("solar", "solar"),
        ("wind", "wind"),
        ("hydro", "hydro"),
    ):
        if kw in name:
            return tag
    if unit.get("fuelType") in FUEL_TAG:
        return FUEL_TAG[unit["fuelType"]]
    for t in (wd or {}).get("types", ()):
        for kw, tag in WD_TAG:
            if kw in t.lower():
                return tag
    return None


def norm(name: str) -> str:
    # Numbers stay: they tell phases apart (Craig 2 vs Craig, Hornsea 1 vs 3).
    # Unit numbers were already stripped by station_name. "01" -> "1".
    # Apostrophes join rather than split ("Fiddler's" == "Fiddlers").
    s = re.sub(r"[\u2019']", "", name.lower().replace("&", " and "))
    s = re.sub(r"[^a-z0-9 ]+", " ", s)
    toks = [t.lstrip("0") or "0" if t.isdigit() else t for t in s.split() if t not in GENERIC]
    return " ".join(toks)


def station_name(units: list[dict]) -> str | None:
    """A station's name from its units' Elexon names, minus unit numbering."""
    names = []
    for u in units:
        n = (u.get("bmUnitName") or "").strip()
        if not n or re.fullmatch(r"[TEMV2I]_+[A-Z0-9-]+", n):
            continue  # Elexon often just repeats the ID
        n = re.sub(
            r"\b(unit|ut|bmu|module|gt|st|stage|phase|block|wf)\b[\s-]*\d*[a-z]?\b",
            " ",
            n,
            flags=re.I,
        )
        names.append(re.sub(r"[\s-]*\d+[a-z]?\s*$", "", n.strip()).strip())
    return max(set(names), key=names.count) if names else None


def best_site(
    stn: str, tag: str | None, cap: float | None, sites: list[dict]
) -> tuple[dict | None, float, str]:
    target = norm(stn)
    if not target:
        return None, 0.0, "no usable name"
    repd_t, dukes_t = TECH.get(tag, (None, None))
    scored = []
    for s in sites:
        if s["status"] not in ("Operational", "Under Construction", "Awaiting Construction"):
            continue
        is_repd = s["src"].startswith("repd")
        if not is_repd and s["tech"] not in DUKES_RELIABLE:
            continue
        allowed = repd_t if is_repd else dukes_t
        if allowed is not None and s["tech"] not in allowed:
            continue
        cand = s.setdefault("_norm", norm(s["name"]))
        if not cand:
            continue
        # Names must match token for token (or near-verbatim). `--validate` showed
        # fuzzy matches put units 40-420 km away: Ilmer Lane -> Illey Lane,
        # Coylton -> Neilston Greener Grid Park, ABRBO -> Burbo.
        if set(target.split()) == set(cand.split()):
            score = 1.0
        else:
            score = difflib.SequenceMatcher(None, target, cand).ratio()
            if score < 0.97:
                continue
        if cap and s["cap"] and not (1 / 3 <= s["cap"] / cap <= 3):
            continue  # same name, wrong size: probably a different site
        bonus = 0.02 * (s["status"] == "Operational") + (
            0.02 if cap and s["cap"] and 0.7 <= s["cap"] / cap <= 1.4 else 0
        )
        scored.append((score + bonus, s))
    if not scored:
        return None, 0.0, "no candidate"
    scored.sort(key=lambda x: -x[0])
    top, s = scored[0]
    # Two different places scoring the same is ambiguous; same place twice is fine.
    rivals = [
        x
        for x in scored[1:]
        if x[0] >= top - 0.03 and km((x[1]["lat"], x[1]["lon"]), (s["lat"], s["lon"])) > 5
    ]
    if rivals:
        return None, top, f"ambiguous: {s['name']} vs {rivals[0][1]['name']}"
    return s, top, "ok"


# ------------------------------------------------------------------ main


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--cache", type=Path, required=True, help="download cache dir")
    ap.add_argument("--out", type=Path, help="SQL migration to write")
    ap.add_argument("--review", type=Path, help="review CSV (default: next to --cache)")
    ap.add_argument(
        "--validate",
        action="store_true",
        help="only report name-match accuracy against known locations",
    )
    args = ap.parse_args()
    if not args.validate and not args.out:
        ap.error("--out is required unless --validate")
    args.cache.mkdir(parents=True, exist_ok=True)
    review_path = args.review or args.cache / "bmu_review.csv"

    print("fetching sources", file=sys.stderr)
    elexon, base, wd = load_elexon(args.cache), load_base(), load_wikidata(args.cache)
    repd, dukes = load_repd(args.cache), load_dukes(args.cache)
    repd_by_id = {s["id"]: s for s in repd}
    sites = repd + dukes

    # Self-check the OSGB conversion: Wikidata vs REPD coords for linked items.
    d = sorted(
        km(next(iter(w["coords"])), (repd_by_id[r]["lat"], repd_by_id[r]["lon"]))
        for w in wd.values()
        if len(w["coords"]) == 1
        for r in w["repd"]
        if r in repd_by_id
    )
    if d:
        print(
            f"OSGB check: Wikidata vs REPD for {len(d)} linked units, "
            f"median {d[len(d) // 2]:.2f} km",
            file=sys.stderr,
        )

    ids = sorted(set(elexon) | set(base))
    stations: dict[str, list[dict]] = defaultdict(list)
    for i in ids:
        stations[STATION_RE.sub("", i)].append(elexon.get(i, {"nationalGridBmUnit": i}))

    if args.validate:
        validate(elexon, base, wd, stations, sites)
        return

    stats: dict[str, int] = defaultdict(int)
    recs = {i: locate_known(i, elexon, base, wd, repd_by_id, stats) for i in ids}
    review: list[dict] = []
    # Name matching runs for generators first. A station-demand unit (CLACD-1,
    # GORDD-1, ...) takes its generator's location (CLAC-1, GORDW-1) when it
    # has one, and is only name-matched itself as a last resort.
    order = sorted(ids, key=lambda i: is_demand(recs[i]))
    for i in order:
        r = recs[i]
        if r["lat"] is not None or not r["single_site"]:
            continue
        if is_demand(r) and inherit_from_generator(r, recs):
            continue
        name_match(r, stations, sites, review)

    out = []
    for i in ids:
        r = recs[i]
        if r["loc_src"]:
            stats["location from " + r["loc_src"].split(":")[0]] += 1
        elif r["single_site"]:
            stats["single-site unit left unlocated"] += 1
        fuel, reg, fuel_src = fuel_type(r, stats)
        out.append(
            (
                i,
                r["bm_unit"],
                fuel,
                reg,
                r["lat"],
                r["lon"],
                r["repd_id"],
                r["u"].get("bmUnitName"),
                r["loc_src"],
                fuel_src,
            )
        )
        stats["units"] += 1
        stats["new units (not in 0004)"] += not r["b"]

    with review_path.open("w", newline="") as f:
        if review:
            wr = csv.DictWriter(f, fieldnames=list(review[0]))
            wr.writeheader()
            wr.writerows(review)
    write_sql(args.out, out)
    for k, n in sorted(stats.items()):
        print(f"  {k:42} {n}", file=sys.stderr)
    print(f"wrote {args.out} and {review_path}", file=sys.stderr)


def locate_known(i, elexon, base, wd, repd_by_id, stats) -> dict:
    """Steps 1-3: existing coordinates, Wikidata by BM unit ID, REPD by ID."""
    u, b, w = elexon.get(i, {}), base.get(i, {}), wd.get(i.upper())
    r = {
        "id": i,
        "u": u,
        "b": b,
        "w": w,
        "site": None,
        "bm_unit": u.get("elexonBmUnit") or b.get("SETT UNIT ID") or None,
        "lat": _num(b.get("latitude")),
        "lon": _num(b.get("longitude")),
        "repd_id": (b.get("UK_REPD_ID") or "").removesuffix(".0") or None,
        "tag": tech_tag(u, w),
    }
    r["loc_src"] = "csv:0004" if r["lat"] is not None else None
    r["single_site"] = (r["bm_unit"] or "")[:2] in ("T_", "E_", "M_")
    if r["lat"] is None and w and w["coords"]:
        pts = sorted(w["coords"])
        if len(pts) == 1 or max(km(pts[0], p) for p in pts) < 2:
            if in_bbox(*pts[0]):
                r["lat"], r["lon"] = pts[0]
                r["loc_src"] = "wikidata:" + ",".join(sorted(w["items"]))
            else:
                stats["wikidata rejected (outside GB bbox)"] += 1
    if w and w["repd"] and not r["repd_id"]:
        r["repd_id"] = sorted(w["repd"])[0]
    if r["lat"] is None and r["repd_id"] in repd_by_id:
        site = r["site"] = repd_by_id[r["repd_id"]]
        r["lat"], r["lon"], r["loc_src"] = site["lat"], site["lon"], f"repd:{r['repd_id']}"
    return r


def is_demand(r: dict) -> bool:
    stn = STATION_RE.sub("", r["id"])
    name = (r["u"].get("bmUnitName") or "").lower()
    return stn.endswith("D") and (
        "demand" in name
        or r["b"].get("DGB FLG") == "Demand"
        or r["u"].get("productionOrConsumptionFlag") == "C"
    )


def inherit_from_generator(r: dict, recs: dict) -> bool:
    """Give a station-demand unit its generator's location: CLACD -> CLAC,
    GORDD -> GORDW, GRIFWD -> GRIFW (same stem, minus or swapping the D)."""
    stn = STATION_RE.sub("", r["id"])
    stem = stn[:-1]
    best = None
    for k, g in recs.items():
        gs = STATION_RE.sub("", k)
        if k == r["id"] or g["lat"] is None or is_demand(g):
            continue
        if gs == stem or (len(gs) == len(stn) and gs[:-1] == stem):
            # Prefer the most trustworthy source: csv > wikidata > repd > a name match.
            rank = ("csv", "wikidata", "repd", "dukes").index(g["loc_src"].split(":")[0])
            if best is None or rank < best[0]:
                best = (rank, k, g)
    if not best:
        return False
    _, k, g = best
    r["lat"], r["lon"], r["loc_src"] = g["lat"], g["lon"], f"generator:{k}"
    r["tag"] = r["tag"] or g["tag"]
    return True


def name_match(r: dict, stations: dict, sites: list[dict], review: list[dict]) -> None:
    """Step 4: station name + technology + capacity against REPD / DUKES."""
    stn = STATION_RE.sub("", r["id"])
    sname = station_name(stations[stn])
    if not sname:
        return
    cap = sum(_num(x.get("generationCapacity")) or 0 for x in stations[stn]) or None
    site, score, why = best_site(sname, r["tag"], cap, sites)
    if site and in_bbox(site["lat"], site["lon"]):
        r["site"] = site
        r["lat"], r["lon"], r["loc_src"] = site["lat"], site["lon"], site["src"]
        r["repd_id"] = r["repd_id"] or site["id"]
    review.append(
        {
            "ngc_bm_unit": r["id"],
            "station_name": sname,
            "tech": r["tag"],
            "station_cap_mw": cap,
            "result": "matched" if r["site"] is site and site else why,
            "score": round(score, 3),
            "site": site["name"] if site else "",
            "site_tech": site["tech"] if site else "",
            "site_cap_mw": site["cap"] if site else "",
            "source": site["src"] if site else "",
            "lat": r["lat"],
            "lon": r["lon"],
        }
    )


def fuel_type(r: dict, stats: dict) -> tuple[str | None, str | None, str | None]:
    """bmrs_fuel_type: Elexon > existing > inferred from the matched technology."""
    u, b, site = r["u"], r["b"], r["site"]
    fuel, fuel_src = u.get("fuelType"), "elexon"
    if not fuel:
        fuel, fuel_src = (b.get("BMRS FUEL TYPE") or None), "csv:0004"
    reg = b.get("REG FUEL TYPE") or None
    tag = r["tag"]
    if site and not tag:
        tag = next((t for t, (rt, dt) in TECH.items() if site["tech"] in rt | dt), None)
    if tag in TAG_FUEL and not reg:
        reg = TAG_FUEL[tag][1]
    if not fuel and tag in TAG_FUEL:
        fuel, fuel_src = TAG_FUEL[tag][0], "inferred:" + tag
    if not fuel:
        fuel_src = None
    if b and u.get("fuelType") and b.get("BMRS FUEL TYPE") and u["fuelType"] != b["BMRS FUEL TYPE"]:
        stats["fuel type changed by Elexon"] += 1
    return fuel, reg, fuel_src


def validate(elexon, base, wd, stations, sites) -> None:
    """Name-match every station whose location is already known (existing or
    Wikidata) and report how far the matched site is from the truth."""
    truth = {}
    for i in set(elexon) | set(base):
        b, w = base.get(i, {}), wd.get(i.upper())
        if b.get("latitude"):
            truth[i] = (float(b["latitude"]), float(b["longitude"]))
        elif w and len(w["coords"]) == 1 and in_bbox(*next(iter(w["coords"]))):
            truth[i] = next(iter(w["coords"]))
    res, bad, done = defaultdict(list), [], set()
    for i, t in truth.items():
        stn = STATION_RE.sub("", i)
        if stn in done or (elexon.get(i, {}).get("elexonBmUnit") or "")[:2] not in (
            "T_",
            "E_",
            "M_",
        ):
            continue
        done.add(stn)
        sname = station_name(stations[stn])
        if not sname:
            continue
        cap = sum(_num(x.get("generationCapacity")) or 0 for x in stations[stn]) or None
        site, _, _ = best_site(sname, tech_tag(elexon.get(i, {}), wd.get(i.upper())), cap, sites)
        if site:
            d = km(t, (site["lat"], site["lon"]))
            res[(site["src"].split(":")[0], site["tech"].split(" ")[0])].append(d)
            if d > 5:
                bad.append((d, i, sname, site["name"], site["src"]))
    print(f"{'source':6} {'tech':12} {'n':>4} {'<=2km':>6} {'<=5km':>6}")
    for (src, tech), ds in sorted(res.items(), key=lambda kv: (kv[0][0], -len(kv[1]))):
        n, near, ok = len(ds), sum(d <= 2 for d in ds), sum(d <= 5 for d in ds)
        print(f"{src:6} {tech:12} {n:4} {near / n:6.0%} {ok / n:6.0%}")
    tot = [d for ds in res.values() for d in ds]
    if tot:
        print(f"overall {len(tot)} matches, {sum(d <= 5 for d in tot) / len(tot):.0%} within 5 km")
    for d, i, sname, site, src in sorted(bad, reverse=True):
        print(f"  >5 km: {i:10} {sname[:28]:28} -> {site[:30]:30} {src:22} {d:6.1f} km")


def q(v) -> str:
    if v is None:
        return "NULL"
    if isinstance(v, float):
        return repr(v)
    return "'" + str(v).replace("'", "''") + "'"


def write_sql(path: Path, rows: list[tuple]) -> None:
    head = f"""-- BMU reference refresh, generated by tools/bmu/refresh_bmu.py. Do not edit by hand.
-- Units and names: Elexon. Fuel type: Elexon > existing > inferred from the
-- matched site's technology. Location: original 0004 coordinates (never
-- touched) > Wikidata P11610 > REPD by ID > the unit's generator (station
-- demand units) > exact name + technology + capacity match to REPD or DUKES
-- 5.11 thermal/nuclear. Anything a previous refresh assigned is recomputed, so
-- re-running a newer refresh corrects it. `location_source` / `fuel_type_source`
-- record which rule applied. {len(rows)} units.

ALTER TABLE bmu ADD COLUMN IF NOT EXISTS bmu_name TEXT;
ALTER TABLE bmu ADD COLUMN IF NOT EXISTS location_source TEXT;
ALTER TABLE bmu ADD COLUMN IF NOT EXISTS fuel_type_source TEXT;
UPDATE bmu SET location_source = 'csv:0004' WHERE latitude IS NOT NULL AND location_source IS NULL;

INSERT INTO bmu (ngc_bm_unit, bm_unit, bmrs_fuel_type, reg_fuel_type, latitude, longitude,
                 repd_id, bmu_name, location_source, fuel_type_source) VALUES
"""
    body = ",\n".join("(" + ", ".join(q(v) for v in r) + ")" for r in rows)
    tail = """
ON CONFLICT (ngc_bm_unit) DO UPDATE SET
  bm_unit          = COALESCE(EXCLUDED.bm_unit, bmu.bm_unit),
  bmrs_fuel_type   = EXCLUDED.bmrs_fuel_type,
  reg_fuel_type    = EXCLUDED.reg_fuel_type,
  latitude         = CASE WHEN bmu.location_source = 'csv:0004' THEN bmu.latitude
                          ELSE EXCLUDED.latitude END,
  longitude        = CASE WHEN bmu.location_source = 'csv:0004' THEN bmu.longitude
                          ELSE EXCLUDED.longitude END,
  location_source  = CASE WHEN bmu.location_source = 'csv:0004' THEN bmu.location_source
                          ELSE EXCLUDED.location_source END,
  repd_id          = EXCLUDED.repd_id,
  bmu_name         = COALESCE(EXCLUDED.bmu_name, bmu.bmu_name),
  fuel_type_source = EXCLUDED.fuel_type_source;
"""
    path.write_text(head + body + tail)


if __name__ == "__main__":
    main()
