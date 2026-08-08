from __future__ import annotations

from datetime import datetime, timedelta

import psycopg

from ..api.client import BMRSClient
from ..api.endpoints import fetch_mels
from .base import datetime_range, run_windows

DATASET = "mels"
TABLE = "mels"
CONFLICT = ["national_grid_bm_unit", "time_from", "notification_sequence"]


def ingest_mels(
    conn: psycopg.Connection,
    client: BMRSClient,
    start: datetime,
    end: datetime,
    chunk: timedelta = timedelta(hours=12),
) -> int:
    return run_windows(
        conn,
        dataset=DATASET,
        table=TABLE,
        conflict_cols=CONFLICT,
        windows=datetime_range(start, end, chunk),
        fetch=lambda w: fetch_mels(client, *w),
        watermark_col="time_from",
    ).rows
