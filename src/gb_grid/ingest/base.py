from __future__ import annotations

from collections import deque
from collections.abc import Callable, Iterable, Iterator, Sequence
from concurrent.futures import Future, ThreadPoolExecutor
from dataclasses import dataclass
from datetime import date, datetime, timedelta
from itertools import islice
from typing import Any, TypeVar

import psycopg
import structlog

from ..config import FETCH_CONCURRENCY
from ..db import set_watermark, upsert

log = structlog.get_logger(__name__)

W = TypeVar("W")


def daterange(start: date, end: date, step_days: int = 1) -> Iterator[tuple[date, date]]:
    cur = start
    while cur <= end:
        nxt = min(cur + timedelta(days=step_days), end + timedelta(days=1))
        yield cur, min(nxt - timedelta(days=1), end)
        cur = nxt


def datetime_range(
    start: datetime, end: datetime, step: timedelta
) -> Iterator[tuple[datetime, datetime]]:
    cur = start
    while cur < end:
        nxt = min(cur + step, end)
        yield cur, nxt
        cur = nxt


@dataclass
class IngestResult:
    rows: int
    last_window_with_data: Any | None = None


def _write(
    conn: psycopg.Connection,
    *,
    dataset: str,
    table: str,
    conflict_cols: Sequence[str],
    rows: list[dict[str, Any]],
    watermark_col: str | None,
) -> int:
    n = upsert(conn, table, rows, conflict_cols)
    if watermark_col and rows:
        latest = max(r[watermark_col] for r in rows if r.get(watermark_col) is not None)
        if isinstance(latest, datetime):
            set_watermark(conn, dataset, latest)
    log.info("ingested", dataset=dataset, rows=n)
    return n


def run_windows(
    conn: psycopg.Connection,
    *,
    dataset: str,
    table: str,
    conflict_cols: list[str],
    windows: Iterable[W],
    fetch: Callable[[W], list[dict[str, Any]]],
    watermark_col: str | None = None,
    concurrency: int | None = None,
) -> IngestResult:
    """Fetch each window, upsert it, advance the watermark.

    Windows are fetched by a pool of ``concurrency`` threads so HTTP latency
    overlaps instead of accumulating — a multi-year backfill is otherwise one
    blocking round-trip per chunk with the database idle throughout. Writes stay
    on the calling thread, in window order, so watermark and last-write-wins
    semantics are unchanged.

    The pool is kept topped up while the caller writes: the next fetch is
    submitted *before* the current batch is upserted, so fetching continues
    during the (serial, and typically slower) database work rather than stopping
    dead for it. Only ``concurrency`` fetches are ever in flight, which bounds
    how many fetched result sets are held in memory at once.
    """
    workers = FETCH_CONCURRENCY if concurrency is None else concurrency
    workers = max(1, workers)
    total = 0
    last_with_data: Any | None = None

    def write(window: W, rows: list[dict[str, Any]]) -> None:
        nonlocal total, last_with_data
        n = _write(
            conn,
            dataset=dataset,
            table=table,
            conflict_cols=conflict_cols,
            rows=rows,
            watermark_col=watermark_col,
        )
        total += n
        if n:
            last_with_data = window

    if workers == 1:
        for window in windows:
            write(window, fetch(window))
        return IngestResult(rows=total, last_window_with_data=last_with_data)

    remaining = iter(windows)
    with ThreadPoolExecutor(max_workers=workers) as pool:
        in_flight: deque[tuple[W, Future[list[dict[str, Any]]]]] = deque(
            (w, pool.submit(fetch, w)) for w in islice(remaining, workers)
        )
        while in_flight:
            window, future = in_flight.popleft()
            rows = future.result()
            nxt = next(remaining, None)
            if nxt is not None:
                in_flight.append((nxt, pool.submit(fetch, nxt)))
            write(window, rows)

    return IngestResult(rows=total, last_window_with_data=last_with_data)
