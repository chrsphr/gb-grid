from __future__ import annotations

from collections.abc import Callable, Iterator
from datetime import date, datetime, timedelta
from typing import Any

import duckdb
import structlog

from ..db import set_watermark, upsert

log = structlog.get_logger(__name__)


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


def run_window(
    conn: duckdb.DuckDBPyConnection,
    *,
    dataset: str,
    table: str,
    conflict_cols: list[str],
    fetch: Callable[[], list[dict[str, Any]]],
    watermark_col: str | None = None,
) -> int:
    """Fetch a window, upsert into table, advance watermark."""
    rows = fetch()
    n = upsert(conn, table, rows, conflict_cols)
    if watermark_col and rows:
        latest = max(r[watermark_col] for r in rows if r.get(watermark_col) is not None)
        if isinstance(latest, datetime):
            set_watermark(conn, dataset, latest)
    log.info("ingested", dataset=dataset, rows=n)
    return n
