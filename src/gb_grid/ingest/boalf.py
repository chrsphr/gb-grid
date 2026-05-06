from __future__ import annotations

from datetime import datetime, timedelta

import psycopg

from ..api.client import BMRSClient
from ..api.endpoints import fetch_boalf
from .base import datetime_range, run_window

DATASET = "boalf"
TABLE = "boalf"
CONFLICT = ["acceptance_id", "time_from"]


def ingest_boalf(
    conn: psycopg.Connection,
    client: BMRSClient,
    start: datetime,
    end: datetime,
    chunk: timedelta = timedelta(hours=6),
) -> int:
    total = 0
    for w_start, w_end in datetime_range(start, end, chunk):
        total += run_window(
            conn,
            dataset=DATASET,
            table=TABLE,
            conflict_cols=CONFLICT,
            fetch=lambda s=w_start, e=w_end: fetch_boalf(client, s, e),
            watermark_col="time_from",
        )
    return total
