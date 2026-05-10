"""Static BMU metadata: fuel type, lat/lon, REPD link.

Sourced from the NESO BMU registry merged with OSUKED bmu-fuel-types and
BEIS REPD; vendored as a CSV alongside this migration. Refresh by replacing
``0004_bmu.csv`` and re-running migrate (the load step upserts).
"""

from __future__ import annotations

import csv
from pathlib import Path

from yoyo import step

CSV_PATH = Path(__file__).with_name("0004_bmu.csv")

CREATE = """
CREATE TABLE IF NOT EXISTS bmu (
    ngc_bm_unit     TEXT PRIMARY KEY,
    bm_unit         TEXT,
    crm_unit_cat    TEXT,
    bmrs_fuel_type  TEXT,
    reg_fuel_type   TEXT,
    reg_type        TEXT,
    dgb_flag        TEXT,
    gc_oc2          TEXT,
    latitude        DOUBLE PRECISION,
    longitude       DOUBLE PRECISION,
    repd_id         TEXT
);
CREATE INDEX IF NOT EXISTS bmu_bm_unit_idx ON bmu (bm_unit);
"""

DROP = "DROP TABLE IF EXISTS bmu;"


def _opt(s: str) -> str | None:
    s = s.strip()
    return s or None


def _float(s: str) -> float | None:
    s = s.strip()
    return float(s) if s else None


def load(conn):
    with CSV_PATH.open(newline="", encoding="utf-8") as f:
        rows = [
            (
                _opt(r["NESO BMU ID"]),
                _opt(r["SETT UNIT ID"]),
                _opt(r["CRM UNIT CAT"]),
                _opt(r["BMRS FUEL TYPE"]),
                _opt(r["REG FUEL TYPE"]),
                _opt(r["REG TYPE"]),
                _opt(r["DGB FLG"]),
                _opt(r["GC OC2"]),
                _float(r["latitude"]),
                _float(r["longitude"]),
                _opt(r["UK_REPD_ID"]),
            )
            for r in csv.DictReader(f)
            if r["NESO BMU ID"].strip()
        ]
    cur = conn.cursor()
    cur.executemany(
        "INSERT INTO bmu (ngc_bm_unit, bm_unit, crm_unit_cat, bmrs_fuel_type, "
        "reg_fuel_type, reg_type, dgb_flag, gc_oc2, latitude, longitude, repd_id) "
        "VALUES (%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s) "
        "ON CONFLICT (ngc_bm_unit) DO UPDATE SET "
        "bm_unit=EXCLUDED.bm_unit, crm_unit_cat=EXCLUDED.crm_unit_cat, "
        "bmrs_fuel_type=EXCLUDED.bmrs_fuel_type, reg_fuel_type=EXCLUDED.reg_fuel_type, "
        "reg_type=EXCLUDED.reg_type, dgb_flag=EXCLUDED.dgb_flag, gc_oc2=EXCLUDED.gc_oc2, "
        "latitude=EXCLUDED.latitude, longitude=EXCLUDED.longitude, repd_id=EXCLUDED.repd_id",
        rows,
    )


steps = [
    step(CREATE, DROP),
    step(load),
]
