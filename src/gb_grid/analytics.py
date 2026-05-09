"""Per-BMU dispatch analytics: combine PN + BOALF into per-minute series.

Definitions used here:

- ``pn_mw``                Final Physical Notification (planned MW). Positive = export.
- ``boa_level_mw``         Dispatched MW after Bid-Offer Acceptances (BOALF).
                           Equal to PN where no acceptance is active.
- ``so_turnup_mw``         max(boa_level_mw - pn_mw, 0). SO instructed unit UP.
- ``boa_curtailment_mw``   max(pn_mw - boa_level_mw, 0). Any acceptance taking unit DOWN.
- ``so_curtailment_mw``    same as ``boa_curtailment_mw`` but only for SO-flagged
                           acceptances (``so_flag = TRUE``).
- ``mel_mw``               Maximum Export Level — physical cap. ``pn_mw`` and
                           ``boa_level_mw`` are both clipped to MEL where MEL is
                           known, since a unit cannot physically exceed it.
"""

from __future__ import annotations

from collections.abc import Iterable
from datetime import datetime

import pandas as pd
import psycopg


def _fetchdf(conn: psycopg.Connection, sql: str, params: tuple) -> pd.DataFrame:
    with conn.cursor() as cur:
        cur.execute(sql, params)
        cols = [d.name for d in cur.description] if cur.description else []
        rows = cur.fetchall()
    return pd.DataFrame(rows, columns=cols)


def _interp_segments(
    df: pd.DataFrame,
    idx: pd.DatetimeIndex,
    value_from: str,
    value_to: str,
    order_cols: list[str] | None = None,
) -> pd.Series:
    """Resample piecewise-linear segments ``[time_from, time_to)`` onto ``idx``.

    Where multiple segments overlap a given index point, the segment that comes
    *later* in ``order_cols`` ascending order wins. For BOALF this is what we
    want: a newer acceptance supersedes any still-running ramp from an older
    one. ``order_cols`` defaults to ``["time_from"]`` (no overlap arbitration).

    Outside any segment -> NaN.
    """
    import numpy as np

    if df.empty:
        return pd.Series(index=idx, dtype="float64")

    sort_keys = order_cols or ["time_from"]
    df = df.sort_values(sort_keys, kind="stable").reset_index(drop=True)

    out = pd.Series(np.nan, index=idx, dtype="float64")
    pts = idx.to_numpy("datetime64[ns]")
    t_from = df["time_from"].to_numpy("datetime64[ns]")
    t_to = df["time_to"].to_numpy("datetime64[ns]")
    v_from = df[value_from].to_numpy("float64")
    v_to = df[value_to].to_numpy("float64")

    # Stamp segments in ascending order so later ones overwrite earlier ones.
    for i in range(len(df)):
        mask = (pts >= t_from[i]) & (pts < t_to[i])
        if not mask.any():
            continue
        span = (t_to[i] - t_from[i]).astype("timedelta64[ns]").astype("int64")
        if span <= 0:
            vals = v_from[i]
        else:
            offset = (pts[mask] - t_from[i]).astype("timedelta64[ns]").astype("int64")
            frac = offset / span
            vals = v_from[i] + (v_to[i] - v_from[i]) * frac
        out.iloc[np.where(mask)[0]] = vals

    return out


def _fetch_pn(
    conn: psycopg.Connection,
    ngc_units: list[str],
    start: datetime,
    end: datetime,
) -> pd.DataFrame:
    return _fetchdf(
        conn,
        """
        SELECT national_grid_bm_unit, time_from, time_to, level_from, level_to
        FROM pn
        WHERE national_grid_bm_unit = ANY(%s)
          AND time_to >= %s AND time_from < %s
        """,
        (ngc_units, start, end),
    )


def _fetch_boalf(
    conn: psycopg.Connection,
    ngc_units: list[str],
    start: datetime,
    end: datetime,
) -> pd.DataFrame:
    # BOALF stores ngc_bm_unit; B1610 fills it via nationalGridBmUnitId. Either column.
    return _fetchdf(
        conn,
        """
        SELECT bm_unit, ngc_bm_unit, acceptance_id, acceptance_time,
               time_from, time_to, level_from, level_to, so_flag
        FROM boalf
        WHERE (ngc_bm_unit = ANY(%s) OR bm_unit = ANY(%s))
          AND time_to >= %s AND time_from < %s
        """,
        (ngc_units, [f"T_{u}" for u in ngc_units] + ngc_units, start, end),
    )


def _fetch_mels(
    conn: psycopg.Connection,
    ngc_units: list[str],
    start: datetime,
    end: datetime,
) -> pd.DataFrame:
    return _fetchdf(
        conn,
        """
        SELECT national_grid_bm_unit, notification_sequence,
               time_from, time_to, level_from, level_to
        FROM mels
        WHERE national_grid_bm_unit = ANY(%s)
          AND time_to >= %s AND time_from < %s
        """,
        (ngc_units, start, end),
    )


def fetch_b1610(
    conn: psycopg.Connection,
    ngc_units: Iterable[str],
    start: datetime,
    end: datetime,
) -> pd.DataFrame:
    """Per-BMU half-hourly actual generation, indexed by half-hour end time.

    Returns columns: ``ts`` (settlement period end time), ``ngc_bm_unit``,
    ``quantity_mw``. The B1610 API reports MWh delivered over the 30-minute
    settlement period; we multiply by 2 to express it as average MW so it
    overlays directly on the PN/BOA series. Settlement period N ends at
    midnight + N*30 minutes (DST-adjusted days are not handled — close
    enough for visual overlay).
    """
    units = list(ngc_units)
    df = _fetchdf(
        conn,
        """
        SELECT ngc_bm_unit, settlement_date, settlement_period, quantity_mwh
        FROM b1610
        WHERE ngc_bm_unit = ANY(%s)
          AND settlement_date >= %s AND settlement_date <= %s
        """,
        (units, start.date(), end.date()),
    )
    if df.empty:
        return df.assign(ts=pd.Series(dtype="datetime64[ns]"), quantity_mw=pd.Series(dtype="float64"))
    df["ts"] = pd.to_datetime(df["settlement_date"]) + pd.to_timedelta(
        df["settlement_period"] * 30, unit="m"
    )
    df["quantity_mw"] = df["quantity_mwh"] * 2.0
    df = df[(df["ts"] >= start) & (df["ts"] <= end)]
    return df[["ts", "ngc_bm_unit", "quantity_mw"]].sort_values(["ngc_bm_unit", "ts"])


def bmu_dispatch(
    conn: psycopg.Connection,
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
    mel = _fetch_mels(conn, units, start, end)

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

        pn_raw = _interp_segments(pn_u, idx, "level_from", "level_to")
        boa_order = ["acceptance_time", "time_from"]
        boa_all = _interp_segments(
            boa_u, idx, "level_from", "level_to", order_cols=boa_order
        )
        boa_so = _interp_segments(
            boa_u[boa_u["so_flag"] == True],  # noqa: E712
            idx,
            "level_from",
            "level_to",
            order_cols=boa_order,
        )
        mel_u = mel[mel["national_grid_bm_unit"] == unit]
        mel_series = _interp_segments(
            mel_u,
            idx,
            "level_from",
            "level_to",
            order_cols=["notification_sequence", "time_from"],
        )

        # MEL is a physical cap — a unit cannot exceed it. Apply to PN and BOA.
        # Where MEL is unknown, leave the value uncapped.
        pn_series = pn_raw.where(mel_series.isna(), pn_raw.clip(upper=mel_series))
        boa_capped = boa_all.where(mel_series.isna(), boa_all.clip(upper=mel_series))
        boa_so_capped = boa_so.where(mel_series.isna(), boa_so.clip(upper=mel_series))

        # Where no acceptance is active, dispatched level == FPN (already MEL-capped).
        boa_level = boa_capped.where(boa_capped.notna(), pn_series)

        delta = boa_level - pn_series
        so_delta = boa_so_capped - pn_series

        df = pd.DataFrame(
            {
                "ts": idx,
                "ngc_bm_unit": unit,
                "pn_mw": pn_series.values,
                "boa_level_mw": boa_level.values,
                "mel_mw": mel_series.values,
                "so_turnup_mw": delta.clip(lower=0).values,
                "boa_curtailment_mw": (-delta).clip(lower=0).values,
                "so_curtailment_mw": (-so_delta).clip(lower=0).fillna(0).values,
            }
        )
        frames.append(df)

    return pd.concat(frames, ignore_index=True) if frames else pd.DataFrame()
