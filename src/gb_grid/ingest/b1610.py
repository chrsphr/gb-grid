from __future__ import annotations

from datetime import UTC, date, datetime, time, timedelta

import psycopg

from ..api.client import BMRSClient
from ..api.endpoints import fetch_b1610
from ..db import refresh_caggs, set_watermark
from .base import daterange, run_windows

DATASET = "b1610"
TABLE = "b1610"
CONFLICT = ["settlement_date", "settlement_period", "bm_unit"]

# Continuous aggregates built on b1610. Their refresh policies only reach back
# ~30 days, so a historical backfill stays invisible to the cagg-backed
# dashboards (the heatmaps, and b1610_series at day zoom) until these are
# refreshed over the loaded window.
B1610_CAGGS = ("b1610_daily_cagg", "b1610_hh_cagg")

# start_offset on both policies in migrations 0009 and 0012. Anything newer than
# this is already refreshed automatically every 30 minutes, so refreshing it here
# too is pure duplicated work on every scheduler tick.
CAGG_POLICY_REACH = timedelta(days=30)


def ingest_b1610(
    conn: psycopg.Connection,
    client: BMRSClient,
    start: date,
    end: date,
    chunk_days: int = 1,
) -> int:
    result = run_windows(
        conn,
        dataset=DATASET,
        table=TABLE,
        conflict_cols=CONFLICT,
        windows=daterange(start, end, step_days=chunk_days),
        fetch=lambda w: fetch_b1610(client, *w),
        watermark_col=None,
    )
    if result.last_window_with_data is not None:
        # B1610 lacks a single timestamp column; advance watermark to end of last covered day.
        last_day = result.last_window_with_data[1]
        set_watermark(conn, DATASET, datetime.combine(last_day, time.max))
        # Fold the loaded rows into the caggs the dashboards read, but only for
        # the part of the window the refresh policies don't already cover. The
        # b1610 hypertable is partitioned on settlement_date (a DATE), so the
        # refresh window is in dates; end is exclusive, hence +1 day.
        policy_floor = datetime.now(UTC).date() - CAGG_POLICY_REACH
        if start < policy_floor:
            refresh_caggs(
                conn, B1610_CAGGS, start, min(end, policy_floor) + timedelta(days=1)
            )
    return result.rows
