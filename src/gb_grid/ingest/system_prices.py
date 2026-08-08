from __future__ import annotations

from datetime import date, datetime, time, timedelta

import psycopg

from ..api.client import BMRSClient
from ..api.endpoints import fetch_system_prices
from ..db import set_watermark
from .base import run_windows

DATASET = "system_prices"
TABLE = "system_prices"
CONFLICT = ["settlement_date", "settlement_period"]


def _days(start: date, end: date) -> list[date]:
    return [start + timedelta(days=i) for i in range((end - start).days + 1)]


def ingest_system_prices(
    conn: psycopg.Connection,
    client: BMRSClient,
    start: date,
    end: date,
) -> int:
    result = run_windows(
        conn,
        dataset=DATASET,
        table=TABLE,
        conflict_cols=CONFLICT,
        windows=_days(start, end),
        fetch=lambda day: fetch_system_prices(client, day),
        watermark_col=None,
    )
    if result.last_window_with_data is not None:
        set_watermark(
            conn, DATASET, datetime.combine(result.last_window_with_data, time.max)
        )
    return result.rows
