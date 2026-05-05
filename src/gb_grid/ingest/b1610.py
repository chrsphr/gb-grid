from __future__ import annotations

from datetime import date, datetime, time

import duckdb

from ..api.client import BMRSClient
from ..api.endpoints import fetch_b1610
from ..db import set_watermark, upsert
from .base import daterange

DATASET = "b1610"
TABLE = "b1610"
CONFLICT = ["settlement_date", "settlement_period", "bm_unit"]


def ingest_b1610(
    conn: duckdb.DuckDBPyConnection,
    client: BMRSClient,
    start: date,
    end: date,
    chunk_days: int = 1,
) -> int:
    total = 0
    for w_start, w_end in daterange(start, end, step_days=chunk_days):
        rows = fetch_b1610(client, w_start, w_end)
        total += upsert(conn, TABLE, rows, CONFLICT)
    if total:
        # B1610 lacks a single timestamp column; advance watermark to end of `end`.
        set_watermark(conn, DATASET, datetime.combine(end, time.max))
    return total
