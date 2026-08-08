from __future__ import annotations

from datetime import datetime, timedelta

import psycopg

from ..api.client import BMRSClient
from ..api.endpoints import fetch_fuelinst
from .base import datetime_range, run_windows

DATASET = "fuelinst"
TABLE = "fuelinst"
CONFLICT = ["publish_time", "fuel_type"]


def ingest_fuelinst(
    conn: psycopg.Connection,
    client: BMRSClient,
    start: datetime,
    end: datetime,
    chunk: timedelta = timedelta(days=1),
) -> int:
    return run_windows(
        conn,
        dataset=DATASET,
        table=TABLE,
        conflict_cols=CONFLICT,
        windows=datetime_range(start, end, chunk),
        fetch=lambda w: fetch_fuelinst(client, *w),
        watermark_col="publish_time",
    ).rows
