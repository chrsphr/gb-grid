from __future__ import annotations

from datetime import date, datetime, time, timedelta

import psycopg

from ..api.client import BMRSClient
from ..api.endpoints import fetch_b1610
from ..db import refresh_caggs, set_watermark
from .base import daterange, run_window

DATASET = "b1610"
TABLE = "b1610"
CONFLICT = ["settlement_date", "settlement_period", "bm_unit"]

# Continuous aggregates built on b1610. Their refresh policies only reach back
# ~30 days, so a historical backfill stays invisible to the cagg-backed
# dashboards (the heatmaps, and b1610_series at day zoom) until these are
# refreshed over the loaded window.
B1610_CAGGS = ("b1610_daily_cagg", "b1610_hh_cagg")


def ingest_b1610(
    conn: psycopg.Connection,
    client: BMRSClient,
    start: date,
    end: date,
    chunk_days: int = 1,
) -> int:
    total = 0
    last_day_with_data: date | None = None
    for w_start, w_end in daterange(start, end, step_days=chunk_days):
        n = run_window(
            conn,
            dataset=DATASET,
            table=TABLE,
            conflict_cols=CONFLICT,
            fetch=lambda s=w_start, e=w_end: fetch_b1610(client, s, e),
            watermark_col=None,
        )
        total += n
        if n:
            last_day_with_data = w_end
    if last_day_with_data is not None:
        # B1610 lacks a single timestamp column; advance watermark to end of last covered day.
        set_watermark(conn, DATASET, datetime.combine(last_day_with_data, time.max))
        # Fold the loaded rows into the caggs the dashboards read. The b1610
        # hypertable is partitioned on settlement_date (a DATE), so the refresh
        # window is in dates; end is exclusive, hence +1 day to include `end`.
        refresh_caggs(conn, B1610_CAGGS, start, end + timedelta(days=1))
    return total
