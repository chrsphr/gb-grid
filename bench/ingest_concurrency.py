"""Where does ingest wall-clock actually go: HTTP fetch, or the database write?

Splits one dataset's backfill into its two phases and times each in isolation,
then times the real `run_windows` path at several concurrency settings. Fetch
concurrency can only ever hide the fetch half, so the serial write time is the
floor — this shows how close to it we get.

Usage (inside `nix develop`):  uv run python bench/ingest_concurrency.py
Upserts are idempotent, so re-running is safe; the first pass is discarded so
every timed pass is an equal all-rows-conflict update.
"""

from __future__ import annotations

import logging
import time
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timedelta

import structlog

from gb_grid.api.client import BMRSClient
from gb_grid.api.endpoints import fetch_boalf
from gb_grid.db import connect, upsert
from gb_grid.ingest.base import datetime_range, run_windows

TABLE = "boalf"
CONFLICT = ["acceptance_id", "time_from"]
START, END = datetime(2026, 6, 3), datetime(2026, 6, 9)
CHUNK = timedelta(hours=6)


def main() -> None:
    structlog.configure(
        wrapper_class=structlog.make_filtering_bound_logger(logging.WARNING)
    )
    windows = list(datetime_range(START, END, CHUNK))
    conn = connect()

    with BMRSClient() as client:
        def fetch(w):
            return fetch_boalf(client, *w)

        fetch(windows[0])  # warm the connection pool
        print(f"{len(windows)} windows, {CHUNK} each\n")

        print("phase isolation")
        batches = None
        for c in (1, 4, 8):
            t0 = time.perf_counter()
            if c == 1:
                batches = [fetch(w) for w in windows]
            else:
                with ThreadPoolExecutor(max_workers=c) as pool:
                    batches = list(pool.map(fetch, windows))
            print(f"  fetch only, concurrency={c}: {time.perf_counter() - t0:6.2f}s")

        t0 = time.perf_counter()
        for b in batches:
            upsert(conn, TABLE, b, CONFLICT)
        rows = sum(len(b) for b in batches)
        print(f"  write only (always serial): {time.perf_counter() - t0:6.2f}s  ({rows} rows)")

        print("\nend to end (run_windows)")
        for c in (1, 2, 4, 8):
            t0 = time.perf_counter()
            run_windows(
                conn,
                dataset="boalf",
                table=TABLE,
                conflict_cols=CONFLICT,
                windows=windows,
                fetch=fetch,
                watermark_col="time_from",
                concurrency=c,
            )
            print(f"  concurrency={c}: {time.perf_counter() - t0:6.2f}s")

    conn.close()


if __name__ == "__main__":
    main()
