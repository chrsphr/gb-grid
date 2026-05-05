"""Per-BMU dispatch analytics: combine PN + BOALF into per-minute series.

Definitions used here:

- ``pn_mw``                Final Physical Notification (planned MW). Positive = export.
- ``boa_level_mw``         Dispatched MW after Bid-Offer Acceptances (BOALF).
                           Equal to PN where no acceptance is active.
- ``so_turnup_mw``         max(boa_level_mw - pn_mw, 0). SO instructed unit UP.
- ``boa_curtailment_mw``   max(pn_mw - boa_level_mw, 0). Any acceptance taking unit DOWN.
- ``so_curtailment_mw``    same as ``boa_curtailment_mw`` but only for SO-flagged
                           acceptances (``so_flag = TRUE``).
"""

from __future__ import annotations

from collections.abc import Iterable
from datetime import datetime

import duckdb
import pandas as pd


def _interp_segments(
    df: pd.DataFrame, idx: pd.DatetimeIndex, value_from: str, value_to: str
) -> pd.Series:
    """Resample piecewise-linear segments [time_from, time_to] onto idx.

    df rows: time_from, time_to, <value_from>, <value_to>.
    Outside any segment -> NaN.
    """
    if df.empty:
        return pd.Series(index=idx, dtype="float64")
    df = df.sort_values("time_from").reset_index(drop=True)
    out = pd.Series(index=idx, dtype="float64")
    # Vectorised lookup: for each idx point find the segment whose [t_from, t_to) covers it.
    t_from = df["time_from"].to_numpy("datetime64[ns]")
    t_to = df["time_to"].to_numpy("datetime64[ns]")
    v_from = df[value_from].to_numpy("float64")
    v_to = df[value_to].to_numpy("float64")
    pts = idx.to_numpy("datetime64[ns]")

    # Right-edge search: segment i covers pts where t_from[i] <= pt < t_to[i].
    import numpy as np

    seg = np.searchsorted(t_from, pts, side="right") - 1
    valid = (seg >= 0) & (seg < len(df))
    if not valid.any():
        return out
    seg_v = seg[valid]
    pts_v = pts[valid]
    in_seg = pts_v < t_to[seg_v]
    seg_v = seg_v[in_seg]
    pts_v = pts_v[in_seg]
    if len(pts_v) == 0:
        return out

    span = (t_to[seg_v] - t_from[seg_v]).astype("timedelta64[ns]").astype("int64")
    offset = (pts_v - t_from[seg_v]).astype("timedelta64[ns]").astype("int64")
    # Avoid div-by-zero: where span == 0, value_from applies.
    with np.errstate(divide="ignore", invalid="ignore"):
        frac = np.where(span > 0, offset / span, 0.0)
    vals = v_from[seg_v] + (v_to[seg_v] - v_from[seg_v]) * frac

    pos_mask = np.zeros(len(pts), dtype=bool)
    pos_mask[np.where(valid)[0][in_seg]] = True
    out.iloc[pos_mask] = vals
    return out


def _fetch_pn(
    conn: duckdb.DuckDBPyConnection,
    ngc_units: list[str],
    start: datetime,
    end: datetime,
) -> pd.DataFrame:
    return conn.execute(
        """
        SELECT national_grid_bm_unit, time_from, time_to, level_from, level_to
        FROM pn
        WHERE national_grid_bm_unit = ANY(?)
          AND time_to >= ? AND time_from < ?
        """,
        [ngc_units, start, end],
    ).df()


def _fetch_boalf(
    conn: duckdb.DuckDBPyConnection,
    ngc_units: list[str],
    start: datetime,
    end: datetime,
) -> pd.DataFrame:
    # BOALF stores ngc_bm_unit; B1610 fills it via nationalGridBmUnitId. Either column.
    return conn.execute(
        """
        SELECT bm_unit, ngc_bm_unit, acceptance_id, time_from, time_to,
               level_from, level_to, so_flag
        FROM boalf
        WHERE (ngc_bm_unit = ANY(?) OR bm_unit = ANY(?))
          AND time_to >= ? AND time_from < ?
        """,
        [ngc_units, [f"T_{u}" for u in ngc_units] + ngc_units, start, end],
    ).df()


def fetch_b1610(
    conn: duckdb.DuckDBPyConnection,
    ngc_units: Iterable[str],
    start: datetime,
    end: datetime,
) -> pd.DataFrame:
    """Per-BMU half-hourly actual generation, indexed by half-hour end time.

    Returns columns: ``ts`` (settlement period end time), ``ngc_bm_unit``,
    ``quantity_mw``. Settlement period N ends at midnight + N*30 minutes
    (DST-adjusted days are not handled — close enough for visual overlay).
    """
    units = list(ngc_units)
    df = conn.execute(
        """
        SELECT ngc_bm_unit, settlement_date, settlement_period, quantity_mw
        FROM b1610
        WHERE ngc_bm_unit = ANY(?)
          AND settlement_date >= ? AND settlement_date <= ?
        """,
        [units, start.date(), end.date()],
    ).df()
    if df.empty:
        return df.assign(ts=pd.Series(dtype="datetime64[ns]"))
    df["ts"] = pd.to_datetime(df["settlement_date"]) + pd.to_timedelta(
        df["settlement_period"] * 30, unit="m"
    )
    df = df[(df["ts"] >= start) & (df["ts"] <= end)]
    return df[["ts", "ngc_bm_unit", "quantity_mw"]].sort_values(["ngc_bm_unit", "ts"])


def bmu_dispatch(
    conn: duckdb.DuckDBPyConnection,
    ngc_units: Iterable[str],
    start: datetime,
    end: datetime,
    freq: str = "1min",
) -> pd.DataFrame:
    """Return a tidy DataFrame with per-minute PN/BOA series for each BMU.

    Columns: ``ts``, ``ngc_bm_unit``, ``pn_mw``, ``boa_level_mw``,
    ``so_turnup_mw``, ``boa_curtailment_mw``, ``so_curtailment_mw``.

    ``boa_level_mw`` falls back to ``pn_mw`` outside any acceptance window
    (i.e. the unit is following its FPN).
    """
    units = list(ngc_units)
    pn = _fetch_pn(conn, units, start, end)
    boa = _fetch_boalf(conn, units, start, end)

    idx = pd.date_range(start=start, end=end, freq=freq, inclusive="left")
    frames: list[pd.DataFrame] = []

    for unit in units:
        pn_u = pn[pn["national_grid_bm_unit"] == unit]
        # BOALF either matches NGC name directly or via T_<NGC>.
        boa_u = boa[
            (boa["ngc_bm_unit"] == unit)
            | (boa["bm_unit"] == unit)
            | (boa["bm_unit"] == f"T_{unit}")
        ]

        pn_series = _interp_segments(pn_u, idx, "level_from", "level_to")
        boa_all = _interp_segments(boa_u, idx, "level_from", "level_to")
        boa_so = _interp_segments(
            boa_u[boa_u["so_flag"] == True],  # noqa: E712
            idx,
            "level_from",
            "level_to",
        )

        # Where no acceptance is active, dispatched level == FPN.
        boa_level = boa_all.where(boa_all.notna(), pn_series)

        delta = boa_level - pn_series
        so_delta = boa_so - pn_series

        df = pd.DataFrame(
            {
                "ts": idx,
                "ngc_bm_unit": unit,
                "pn_mw": pn_series.values,
                "boa_level_mw": boa_level.values,
                "so_turnup_mw": delta.clip(lower=0).values,
                "boa_curtailment_mw": (-delta).clip(lower=0).values,
                "so_curtailment_mw": (-so_delta).clip(lower=0).fillna(0).values,
            }
        )
        frames.append(df)

    return pd.concat(frames, ignore_index=True) if frames else pd.DataFrame()
