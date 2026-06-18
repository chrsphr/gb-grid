"""Pipeline benchmark harness: vanilla Postgres vs TimescaleDB.

Times the operations Timescale could plausibly change (daily rollup, dashboard
range-scan reads, storage), plus the pandas dispatch compute as a control that
should *not* move. Run against any DB URL; emits JSON for before/after diffing.

    python bench/bench.py --label vanilla
    python bench/bench.py --label timescale --rollup cagg --db "$GB_GRID_DATABASE_URL"

Compare two runs:
    python bench/bench.py --compare bench/results/vanilla.json bench/results/timescale.json
"""

from __future__ import annotations

import argparse
import json
import os
import statistics
import time
from datetime import timedelta
from pathlib import Path
from typing import Any

import psycopg

RESULTS_DIR = Path(__file__).parent / "results"

# Dashboard queries lifted verbatim from grafana/dashboards/*.json, with the
# Grafana macros ($__timeFrom/$__timeTo/$station/$constraint_group) replaced by
# psycopg params. Names mirror the panels they back.
READS: dict[str, str] = {
    "disp_station_range": """
        SELECT d.ts AS time, SUM(d.pn_mw) AS pn, SUM(d.boa_level_mw) AS dispatched,
               SUM(d.mel_mw) AS mel
        FROM bmu_dispatch d JOIN bmu b ON b.ngc_bm_unit = d.bmu
        WHERE b.station = %(station)s AND d.ts BETWEEN %(start)s AND %(end)s
        GROUP BY d.ts ORDER BY d.ts
    """,
    "disp_station_curtailment": """
        SELECT d.ts AS time, SUM(d.so_turnup_mw) AS turnup,
               -SUM(d.boa_curtailment_mw) AS boa_curt, -SUM(d.so_curtailment_mw) AS so_curt
        FROM bmu_dispatch d JOIN bmu b ON b.ngc_bm_unit = d.bmu
        WHERE b.station = %(station)s AND d.ts BETWEEN %(start)s AND %(end)s
        GROUP BY d.ts ORDER BY d.ts
    """,
    "annual_summary_pn": """
        SELECT date_trunc('week', d.date)::timestamp AS time, to_char(d.date, 'ID') AS metric,
               SUM(d.pn_mwh) AS value
        FROM bmu_dispatch_daily d JOIN bmu b ON b.ngc_bm_unit = d.bmu
        WHERE b.station = %(station)s AND d.date BETWEEN %(start_d)s AND %(end_d)s
        GROUP BY date_trunc('week', d.date), d.date ORDER BY time, metric
    """,
    "b1610_station": """
        SELECT (a.settlement_date::timestamp + ((a.settlement_period - 1) * INTERVAL '30 minutes')) AS time,
               SUM(a.quantity_mwh) * 2 AS actual
        FROM b1610 a JOIN bmu b ON b.ngc_bm_unit = a.ngc_bm_unit
        WHERE b.station = %(station)s
          AND (a.settlement_date::timestamp + ((a.settlement_period - 1) * INTERVAL '30 minutes'))
              BETWEEN %(start)s AND %(end)s
        GROUP BY 1 ORDER BY 1
    """,
    "constraints_window": """
        SELECT ts AS time, flow_mw AS value,
               AVG(flow_mw) OVER (ORDER BY ts ROWS BETWEEN 47 PRECEDING AND CURRENT ROW) AS avg24h,
               MAX(flow_mw) OVER (PARTITION BY date_trunc('day', ts)) AS daymax
        FROM constraints
        WHERE constraint_group = %(cgroup)s AND ts BETWEEN %(start)s AND %(end)s
        ORDER BY time
    """,
}


def _median_ms(conn: psycopg.Connection, sql: str, params: dict, repeats: int) -> dict[str, Any]:
    rows = 0
    times: list[float] = []
    for _ in range(repeats):
        with conn.cursor() as cur:
            t0 = time.perf_counter()
            cur.execute(sql, params)
            fetched = cur.fetchall()
            times.append((time.perf_counter() - t0) * 1000)
            rows = len(fetched)
    return {"median_ms": round(statistics.median(times), 2),
            "min_ms": round(min(times), 2), "rows": rows}


def _pick_params(conn: psycopg.Connection) -> dict[str, Any]:
    with conn.cursor() as cur:
        cur.execute("SELECT min(ts), max(ts) FROM bmu_dispatch")
        first, last = cur.fetchone()
        # Busiest station = most dispatch rows in the last day (a realistic dashboard load).
        cur.execute("""
            SELECT b.station, count(*) c
            FROM bmu_dispatch d JOIN bmu b ON b.ngc_bm_unit = d.bmu
            WHERE d.ts >= %s AND b.station IS NOT NULL
            GROUP BY b.station ORDER BY c DESC LIMIT 1
        """, (last - timedelta(days=1),))
        station = cur.fetchone()[0]
        cur.execute("""
            SELECT constraint_group, count(*) c FROM constraints
            GROUP BY constraint_group ORDER BY c DESC LIMIT 1
        """)
        cgroup = cur.fetchone()[0]
    return {
        "station": station, "cgroup": cgroup,
        "first": first, "last": last,
        # 7-day window ending at the latest data.
        "start": last - timedelta(days=7), "end": last,
        "start_d": first.date(), "end_d": last.date(),
    }


TS_TABLES = ("bmu_dispatch", "mels", "pn", "b1610", "constraints", "boalf", "fuelinst")


def _storage(conn: psycopg.Connection) -> dict[str, int]:
    """Total on-disk size per table. For hypertables, pg_total_relation_size on
    the parent excludes the chunks, so use hypertable_size() instead."""
    with conn.cursor() as cur:
        cur.execute("SELECT extname FROM pg_extension WHERE extname='timescaledb'")
        has_ts = cur.fetchone() is not None
        hyper: set[str] = set()
        if has_ts:
            cur.execute("SELECT hypertable_name FROM timescaledb_information.hypertables")
            hyper = {r[0] for r in cur.fetchall()}
        out: dict[str, int] = {}
        for t in TS_TABLES:
            if t in hyper:
                cur.execute("SELECT hypertable_size(%s)", (t,))
            else:
                cur.execute("SELECT pg_total_relation_size(%s)", (t,))
            out[t] = int(cur.fetchone()[0])
        return out


def _rollup_sql(conn: psycopg.Connection, p: dict) -> dict[str, Any]:
    """Vanilla path: the hand-rolled trailing-14-day daily rollup."""
    from gb_grid.materialize import materialize_dispatch_daily
    start_d = p["end_d"] - timedelta(days=13)
    t0 = time.perf_counter()
    n = materialize_dispatch_daily(conn, start_d, p["end_d"])
    return {"median_ms": round((time.perf_counter() - t0) * 1000, 2), "rows": n}


def _rollup_cagg(conn: psycopg.Connection, p: dict) -> dict[str, Any]:
    """Timescale path: incremental refresh of the daily continuous aggregates."""
    start_d = p["end_d"] - timedelta(days=13)
    t0 = time.perf_counter()
    # CALL cannot run inside a txn block; autocommit conn is fine.
    for cagg in ("bmu_dispatch_daily_cagg", "b1610_daily_cagg"):
        conn.execute(f"CALL refresh_continuous_aggregate('{cagg}', %s, %s)",
                     (start_d, p["end_d"] + timedelta(days=1)))
    return {"median_ms": round((time.perf_counter() - t0) * 1000, 2), "rows": None}


def _pandas_dispatch(conn: psycopg.Connection, p: dict) -> dict[str, Any]:
    """Control: pure pandas interpolation cost for one busy station over a day.

    Reads pn/boalf/mels and interpolates — the genuinely CPU-bound step that
    Timescale is not expected to change (beyond marginal read speedups)."""
    from gb_grid.analytics import bmu_dispatch
    with conn.cursor() as cur:
        cur.execute("SELECT ngc_bm_unit FROM bmu WHERE station = %s AND ngc_bm_unit IS NOT NULL",
                    (p["station"],))
        units = [r[0] for r in cur.fetchall()]
    start = p["last"] - timedelta(days=1)
    t0 = time.perf_counter()
    df = bmu_dispatch(conn, units, start, p["last"], freq="5min")
    return {"median_ms": round((time.perf_counter() - t0) * 1000, 2),
            "rows": len(df), "units": len(units)}


def run(label: str, db: str, rollup: str, repeats: int) -> dict[str, Any]:
    conn = psycopg.connect(db, autocommit=True)
    try:
        p = _pick_params(conn)
        with conn.cursor() as cur:
            cur.execute("SELECT extversion FROM pg_extension WHERE extname='timescaledb'")
            r = cur.fetchone()
        result: dict[str, Any] = {
            "label": label,
            "timescaledb": r[0] if r else None,
            "params": {"station": p["station"], "cgroup": p["cgroup"],
                       "window_days": 7, "first": str(p["first"]), "last": str(p["last"])},
            "reads": {}, "storage": {},
        }
        # Warm the cache once, then time.
        for name, sql in READS.items():
            _median_ms(conn, sql, p, 1)
            result["reads"][name] = _median_ms(conn, sql, p, repeats)
        result["pandas_dispatch"] = _pandas_dispatch(conn, p)
        result["rollup"] = (_rollup_cagg if rollup == "cagg" else _rollup_sql)(conn, p)
        result["storage"] = _storage(conn)
        result["storage_total"] = sum(result["storage"].values())
    finally:
        conn.close()
    return result


def _fmt_bytes(n: int) -> str:
    for unit in ("B", "KB", "MB", "GB"):
        if n < 1024:
            return f"{n:.0f} {unit}"
        n /= 1024
    return f"{n:.1f} TB"


def compare(a_path: str, b_path: str) -> None:
    a = json.loads(Path(a_path).read_text())
    b = json.loads(Path(b_path).read_text())
    print(f"\n  {a['label']} (ts={a['timescaledb']})  vs  {b['label']} (ts={b['timescaledb']})\n")
    print(f"  {'operation':<28}{a['label']:>14}{b['label']:>14}{'change':>12}")
    print("  " + "-" * 66)

    def line(name: str, av: float, bv: float, unit: str = "ms") -> None:
        if av and bv:
            pct = (bv - av) / av * 100
            speed = f"{av / bv:.2f}x" if bv else "-"
            tag = f"{pct:+.0f}% ({speed})"
        else:
            tag = "-"
        af = _fmt_bytes(av) if unit == "B" else f"{av:.1f}"
        bf = _fmt_bytes(bv) if unit == "B" else f"{bv:.1f}"
        print(f"  {name:<28}{af:>14}{bf:>14}{tag:>12}")

    line("rollup (14d)", a["rollup"]["median_ms"], b["rollup"]["median_ms"])
    for name in a["reads"]:
        line(f"read:{name}", a["reads"][name]["median_ms"], b["reads"][name]["median_ms"])
    line("pandas_dispatch (control)", a["pandas_dispatch"]["median_ms"],
         b["pandas_dispatch"]["median_ms"])
    print("  " + "-" * 66)
    line("storage total", a["storage_total"], b["storage_total"], unit="B")
    print()


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--label", default="run")
    ap.add_argument("--db", default=os.environ.get("GB_GRID_DATABASE_URL"))
    ap.add_argument("--rollup", choices=["sql", "cagg"], default="sql")
    ap.add_argument("--repeats", type=int, default=5)
    ap.add_argument("--compare", nargs=2, metavar=("A", "B"))
    args = ap.parse_args()

    if args.compare:
        compare(*args.compare)
        return

    if not args.db:
        raise SystemExit("set --db or GB_GRID_DATABASE_URL")
    result = run(args.label, args.db, args.rollup, args.repeats)
    RESULTS_DIR.mkdir(exist_ok=True)
    out = RESULTS_DIR / f"{args.label}.json"
    out.write_text(json.dumps(result, indent=2, default=str))
    print(f"wrote {out}")
    print(json.dumps(result, indent=2, default=str))


if __name__ == "__main__":
    main()
