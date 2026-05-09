"""Materialize per-minute BMU dispatch series into ``bmu_dispatch``.

Wraps :func:`gb_grid.analytics.bmu_dispatch` for any BMU with PN activity in
the requested window, then upserts the rows. This is what Grafana queries —
the heavy pandas work happens here, not at dashboard render time.
"""

from __future__ import annotations

from datetime import datetime

import psycopg
import structlog

from .analytics import bmu_dispatch
from .db import upsert

log = structlog.get_logger(__name__)

TABLE = "bmu_dispatch"
CONFLICT_COLS = ("bmu", "ts")


def _active_bmus(conn: psycopg.Connection, start: datetime, end: datetime) -> list[str]:
    """BMUs with at least one BOA acceptance in the window.

    Most BMUs sit on their FPN with no acceptance — for those the materialized
    series adds no information beyond the raw ``pn`` table, so we skip them.
    BOALF stores either ``ngc_bm_unit`` (preferred) or ``bm_unit`` (often
    prefixed ``T_<NGC>``); we coalesce both into the NGC name used by ``pn``.
    """
    with conn.cursor() as cur:
        cur.execute(
            """
            SELECT DISTINCT
              COALESCE(ngc_bm_unit, regexp_replace(bm_unit, '^T_', ''))
            FROM boalf
            WHERE time_to >= %s AND time_from < %s
            """,
            (start, end),
        )
        return [r[0] for r in cur.fetchall() if r[0]]


def materialize_dispatch(
    conn: psycopg.Connection,
    start: datetime,
    end: datetime,
    bmus: list[str] | None = None,
    freq: str = "5min",
) -> int:
    """Compute per-minute dispatch for ``bmus`` (or all active) and upsert.

    Returns the number of rows written.
    """
    units = bmus or _active_bmus(conn, start, end)
    if not units:
        log.info("materialize_dispatch_no_bmus", start=start, end=end)
        return 0

    df = bmu_dispatch(conn, units, start, end, freq=freq)
    if df.empty:
        return 0

    df = df.dropna(subset=["pn_mw", "boa_level_mw"], how="all")
    rows = [
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
    n = upsert(conn, TABLE, rows, CONFLICT_COLS)
    log.info("materialize_dispatch_wrote", rows=n, units=len(units), start=start, end=end)
    return n
