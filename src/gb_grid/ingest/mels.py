from __future__ import annotations

from datetime import datetime, timedelta

import duckdb

from ..api.client import BMRSClient
from ..api.endpoints import fetch_mels
from .base import datetime_range, run_window

DATASET = "mels"
TABLE = "mels"
CONFLICT = ["national_grid_bm_unit", "time_from", "notification_sequence"]


def ingest_mels(
    conn: duckdb.DuckDBPyConnection,
    client: BMRSClient,
    start: datetime,
    end: datetime,
    chunk: timedelta = timedelta(hours=12),
) -> int:
    total = 0
    for w_start, w_end in datetime_range(start, end, chunk):
        total += run_window(
            conn,
            dataset=DATASET,
            table=TABLE,
            conflict_cols=CONFLICT,
            fetch=lambda s=w_start, e=w_end: fetch_mels(client, s, e),
            watermark_col="time_from",
        )
    return total
