"""Compare the two upsert paths: row-wise executemany vs COPY into staging.

Usage (inside `nix develop`):  uv run python bench/upsert_paths.py
Writes into a throwaway table, so it is safe to run against the dev database.
"""

from __future__ import annotations

import os
import time
from datetime import datetime, timedelta

import psycopg

from gb_grid.db import _upsert_copy, _upsert_values

TABLE = "_bench_upsert"
CONFLICT = ["publish_time", "fuel_type"]
FUELS = ["WIND", "GAS", "NUCLEAR", "COAL", "SOLAR", "BIOMASS", "HYDRO", "OTHER"]
BASE = datetime(2026, 1, 1)


def make_rows(n: int) -> list[dict]:
    return [
        {
            "publish_time": BASE + timedelta(minutes=5 * (i // len(FUELS))),
            "fuel_type": FUELS[i % len(FUELS)],
            "generation_mw": float(i % 5000),
        }
        for i in range(n)
    ]


def setup(conn: psycopg.Connection, hypertable: bool) -> None:
    with conn.cursor() as cur:
        cur.execute(f"DROP TABLE IF EXISTS {TABLE}")
        cur.execute(
            f"CREATE TABLE {TABLE} ("
            "  publish_time TIMESTAMP NOT NULL,"
            "  fuel_type TEXT NOT NULL,"
            "  generation_mw DOUBLE PRECISION,"
            "  PRIMARY KEY (publish_time, fuel_type))"
        )
        if hypertable:
            cur.execute(
                f"SELECT create_hypertable('{TABLE}', 'publish_time', "
                "chunk_time_interval => INTERVAL '1 day')"
            )


def time_path(conn: psycopg.Connection, fn, rows: list[dict], preload: bool) -> float:
    with conn.cursor() as cur:
        cur.execute(f"TRUNCATE {TABLE}")
    if preload:  # every row will collide: the backfill-overlap / revision case
        _upsert_copy(conn, TABLE, rows, CONFLICT)
    t0 = time.perf_counter()
    fn(conn, TABLE, rows, CONFLICT)
    return time.perf_counter() - t0


def main() -> None:
    conn = psycopg.connect(os.environ["GB_GRID_DATABASE_URL"], autocommit=True)
    for hypertable in (False, True):
        for preload in (False, True):
            setup(conn, hypertable)
            kind = "hypertable" if hypertable else "plain table"
            case = "all rows conflict" if preload else "empty target"
            print(f"\n{kind}, {case}")
            print(f"{'rows':>9} {'executemany':>13} {'copy':>10} {'speedup':>9}")
            for n in (500, 5_000, 50_000, 250_000):
                rows = make_rows(n)
                values_s = time_path(conn, _upsert_values, rows, preload)
                copy_s = time_path(conn, _upsert_copy, rows, preload)
                print(
                    f"{n:>9,} {values_s:>12.3f}s {copy_s:>9.3f}s "
                    f"{values_s / copy_s:>8.1f}x"
                )
    with conn.cursor() as cur:
        cur.execute(f"DROP TABLE IF EXISTS {TABLE}")
    conn.close()


if __name__ == "__main__":
    main()
