"""Concurrent window fetching must not change ordering or watermark semantics."""

import threading
from datetime import datetime, timedelta

from gb_grid.db import get_watermark
from gb_grid.ingest.base import run_windows

BASE = datetime(2026, 5, 1, 0, 0)
WINDOWS = [(BASE + timedelta(hours=i), BASE + timedelta(hours=i + 1)) for i in range(8)]


def _fetch(window):
    start, _ = window
    return [
        {
            "publish_time": start,
            "settlement_date": None,
            "settlement_period": 1,
            "fuel_type": "WIND",
            "generation_mw": float(start.hour),
        }
    ]


def _run(db, fetch, concurrency, windows=WINDOWS):
    return run_windows(
        db,
        dataset="fuelinst",
        table="fuelinst",
        conflict_cols=["publish_time", "fuel_type"],
        windows=iter(windows),
        fetch=fetch,
        watermark_col="publish_time",
        concurrency=concurrency,
    )


def test_concurrent_matches_serial(db):
    serial = _run(db, _fetch, concurrency=1)
    with db.cursor() as cur:
        cur.execute("TRUNCATE fuelinst")
    parallel = _run(db, _fetch, concurrency=4)

    assert serial.rows == parallel.rows == len(WINDOWS)
    assert serial.last_window_with_data == parallel.last_window_with_data == WINDOWS[-1]
    assert get_watermark(db, "fuelinst") == WINDOWS[-1][0]


def test_fetches_actually_run_concurrently(db):
    """Guards against the pool silently degrading to serial execution."""
    barrier = threading.Barrier(4, timeout=10)

    def fetch(window):
        barrier.wait()  # raises BrokenBarrierError unless 4 run at once
        return _fetch(window)

    result = _run(db, fetch, concurrency=4)
    assert result.rows == len(WINDOWS)


def test_last_window_with_data_skips_empty_tail(db):
    def fetch(window):
        return [] if window[0].hour >= 5 else _fetch(window)

    result = _run(db, fetch, concurrency=4)
    assert result.rows == 5
    assert result.last_window_with_data == WINDOWS[4]


def test_writes_stay_ordered_under_concurrency(db):
    """Later windows must win on conflict, whatever order the fetches finish in."""

    def fetch(window):
        row = _fetch(window)[0]
        return [{**row, "publish_time": BASE, "generation_mw": float(window[0].hour)}]

    _run(db, fetch, concurrency=4)
    with db.cursor() as cur:
        cur.execute("SELECT generation_mw FROM fuelinst")
        assert cur.fetchall() == [(float(WINDOWS[-1][0].hour),)]
