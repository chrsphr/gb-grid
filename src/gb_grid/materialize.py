"""Materialize per-minute BMU dispatch series into ``bmu_dispatch``.

Wraps :func:`gb_grid.analytics.bmu_dispatch` for any BMU with PN activity in
the requested window, then upserts the rows. This is what Grafana queries —
the heavy pandas work happens here, not at dashboard render time.
"""

from __future__ import annotations

import os
from concurrent.futures import ProcessPoolExecutor, as_completed
from datetime import datetime
from typing import Any

import psycopg
import structlog

from .analytics import bmu_dispatch
from .db import connect, upsert

log = structlog.get_logger(__name__)

TABLE = "bmu_dispatch"
CONFLICT_COLS = ("bmu", "ts")


def _active_bmus(conn: psycopg.Connection, start: datetime, end: datetime) -> list[str]:
    """BMUs with PN data in the window — i.e. essentially every BMU.

    Even units that never receive a BOA are worth materializing so the
    dashboard can compare planned (PN) vs actual (B1610) generation.
    """
    with conn.cursor() as cur:
        cur.execute(
            """
            SELECT DISTINCT national_grid_bm_unit
            FROM pn
            WHERE time_to >= %s AND time_from < %s
            """,
            (start, end),
        )
        return [r[0] for r in cur.fetchall() if r[0]]


def _rows_for_units(
    units: list[str], start: datetime, end: datetime, freq: str
) -> list[dict[str, Any]]:
    """Worker entry point: compute dispatch for ``units`` and return upsert rows."""
    conn = connect()
    try:
        df = bmu_dispatch(conn, units, start, end, freq=freq)
    finally:
        conn.close()
    if df.empty:
        return []
    df = df.dropna(subset=["pn_mw", "boa_level_mw"], how="all")
    return [
        {
            "bmu": r.ngc_bm_unit,
            "ts": r.ts.to_pydatetime(),
            "pn_mw": None if r.pn_mw != r.pn_mw else float(r.pn_mw),  # NaN check
            "boa_level_mw": None if r.boa_level_mw != r.boa_level_mw else float(r.boa_level_mw),
            "mel_mw": None if r.mel_mw != r.mel_mw else float(r.mel_mw),
            "so_turnup_mw": float(r.so_turnup_mw),
            "boa_curtailment_mw": float(r.boa_curtailment_mw),
            "so_curtailment_mw": float(r.so_curtailment_mw),
        }
        for r in df.itertuples(index=False)
    ]


def materialize_dispatch(
    conn: psycopg.Connection,
    start: datetime,
    end: datetime,
    bmus: list[str] | None = None,
    freq: str = "5min",
    workers: int | None = None,
) -> int:
    """Compute per-minute dispatch for ``bmus`` (or all active) and upsert.

    Splits BMUs across a process pool — the per-segment numpy work in
    ``_interp_segments`` is pure-Python overhead-bound, so threads don't help.
    Returns the number of rows written.
    """
    units = bmus or _active_bmus(conn, start, end)
    if not units:
        log.info("materialize_dispatch_no_bmus", start=start, end=end)
        return 0

    n_workers = workers or max(1, (os.cpu_count() or 2) - 1)
    n_workers = min(n_workers, len(units))

    # Round-robin so each worker sees a mix of heavy/light BMUs rather than
    # one shard getting all the alphabetically-clustered baseload units.
    chunks = [units[i::n_workers] for i in range(n_workers)]

    total = 0
    if n_workers == 1:
        rows = _rows_for_units(units, start, end, freq)
        total = upsert(conn, TABLE, rows, CONFLICT_COLS)
    else:
        with ProcessPoolExecutor(max_workers=n_workers) as pool:
            futures = [pool.submit(_rows_for_units, c, start, end, freq) for c in chunks]
            for fut in as_completed(futures):
                rows = fut.result()
                if rows:
                    total += upsert(conn, TABLE, rows, CONFLICT_COLS)

    log.info(
        "materialize_dispatch_wrote",
        rows=total,
        units=len(units),
        workers=n_workers,
        start=start,
        end=end,
    )
    return total


# bmu_dispatch is stored at 5-minute resolution (see PollConfig + materialize_dispatch
# default freq). Each sample therefore represents 5/60 hours of energy.
_DAILY_INTERVAL_HOURS = 5.0 / 60.0


def materialize_dispatch_daily(
    conn: psycopg.Connection,
    start_date: Any,
    end_date: Any,
) -> int:
    """Roll ``bmu_dispatch`` (and ``b1610``) up to per-BMU daily MWh totals.

    Pure SQL upsert — fast enough to recompute the whole year on demand.
    ``start_date`` and ``end_date`` are inclusive ``date`` objects.
    """
    sql = """
    WITH dispatch AS (
        SELECT bmu,
               (ts AT TIME ZONE 'UTC')::date AS date,
               SUM(pn_mw)              * %(h)s AS pn_mwh,
               SUM(boa_level_mw)       * %(h)s AS boa_dispatched_mwh,
               SUM(so_turnup_mw)       * %(h)s AS so_turnup_mwh,
               SUM(boa_curtailment_mw) * %(h)s AS boa_curtailment_mwh,
               SUM(so_curtailment_mw)  * %(h)s AS so_curtailment_mwh
        FROM bmu_dispatch
        WHERE ts >= %(start)s AND ts < %(end)s
        GROUP BY bmu, (ts AT TIME ZONE 'UTC')::date
    ),
    actual AS (
        SELECT ngc_bm_unit AS bmu,
               settlement_date  AS date,
               SUM(quantity_mwh) AS b1610_mwh
        FROM b1610
        WHERE settlement_date >= %(start_d)s AND settlement_date <= %(end_d)s
          AND ngc_bm_unit IS NOT NULL
        GROUP BY ngc_bm_unit, settlement_date
    )
    INSERT INTO bmu_dispatch_daily (
        bmu, date, pn_mwh, boa_dispatched_mwh,
        so_turnup_mwh, boa_curtailment_mwh, so_curtailment_mwh, b1610_mwh
    )
    SELECT COALESCE(d.bmu, a.bmu)              AS bmu,
           COALESCE(d.date, a.date)            AS date,
           d.pn_mwh, d.boa_dispatched_mwh,
           d.so_turnup_mwh, d.boa_curtailment_mwh, d.so_curtailment_mwh,
           a.b1610_mwh
    FROM dispatch d
    FULL OUTER JOIN actual a ON a.bmu = d.bmu AND a.date = d.date
    ON CONFLICT (bmu, date) DO UPDATE SET
        pn_mwh              = EXCLUDED.pn_mwh,
        boa_dispatched_mwh  = EXCLUDED.boa_dispatched_mwh,
        so_turnup_mwh       = EXCLUDED.so_turnup_mwh,
        boa_curtailment_mwh = EXCLUDED.boa_curtailment_mwh,
        so_curtailment_mwh  = EXCLUDED.so_curtailment_mwh,
        b1610_mwh           = EXCLUDED.b1610_mwh
    """
    # bmu_dispatch is range-scanned by ts; b1610 is keyed on settlement_date.
    # Use end-of-day on `end_date` so a full inclusive day window catches everything.
    from datetime import datetime, time, timedelta

    start_dt = datetime.combine(start_date, time.min)
    end_dt = datetime.combine(end_date + timedelta(days=1), time.min)

    with conn.cursor() as cur:
        cur.execute(
            sql,
            {
                "h": _DAILY_INTERVAL_HOURS,
                "start": start_dt,
                "end": end_dt,
                "start_d": start_date,
                "end_d": end_date,
            },
        )
        n = cur.rowcount

    log.info(
        "materialize_dispatch_daily_wrote",
        rows=n,
        start=start_date,
        end=end_date,
    )
    return n
