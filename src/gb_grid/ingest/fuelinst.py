from __future__ import annotations

from datetime import datetime, timedelta

import duckdb

from ..api.client import BMRSClient
from ..api.endpoints import fetch_fuelinst
from .base import datetime_range, run_window

DATASET = "fuelinst"
TABLE = "fuelinst"
CONFLICT = ["publish_time", "fuel_type"]


def ingest_fuelinst(
    conn: duckdb.DuckDBPyConnection,
    client: BMRSClient,
    start: datetime,
    end: datetime,
    chunk: timedelta = timedelta(days=1),
) -> int:
    total = 0
    for w_start, w_end in datetime_range(start, end, chunk):
        total += run_window(
            conn,
            dataset=DATASET,
            table=TABLE,
            conflict_cols=CONFLICT,
            fetch=lambda s=w_start, e=w_end: fetch_fuelinst(client, s, e),
            watermark_col="publish_time",
        )
    return total
