from __future__ import annotations

from datetime import date, datetime, time, timedelta

import duckdb

from ..api.client import BMRSClient
from ..api.endpoints import fetch_system_prices
from ..db import set_watermark, upsert

DATASET = "system_prices"
TABLE = "system_prices"
CONFLICT = ["settlement_date", "settlement_period"]


def ingest_system_prices(
    conn: duckdb.DuckDBPyConnection,
    client: BMRSClient,
    start: date,
    end: date,
) -> int:
    total = 0
    cur = start
    one = timedelta(days=1)
    last_day_with_data: date | None = None
    while cur <= end:
        rows = fetch_system_prices(client, cur)
        n = upsert(conn, TABLE, rows, CONFLICT)
        total += n
        if n:
            last_day_with_data = cur
        cur += one
    if last_day_with_data is not None:
        set_watermark(conn, DATASET, datetime.combine(last_day_with_data, time.max))
    return total
