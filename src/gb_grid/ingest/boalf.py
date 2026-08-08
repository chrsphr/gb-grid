from __future__ import annotations

from datetime import datetime, timedelta

import psycopg

from ..api.client import BMRSClient
from ..api.endpoints import fetch_boalf
from .base import datetime_range, run_windows

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
    return run_windows(
        conn,
        dataset=DATASET,
        table=TABLE,
        conflict_cols=CONFLICT,
        windows=datetime_range(start, end, chunk),
        fetch=lambda w: fetch_boalf(client, *w),
        watermark_col="time_from",
    ).rows
