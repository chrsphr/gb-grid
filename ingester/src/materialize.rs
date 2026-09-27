//! Per-BMU dispatch materialization: combine PN + BOALF (+ MEL) segments onto a
//! fixed grid, then upsert into `bmu_dispatch` and refresh its caggs.
//!
//! IMPLEMENT THIS MODULE. Do **not** change the public items below — they are
//! consumed by `scheduler.rs` and `main.rs`. You may add private helpers,
//! including the `bmu_dispatch` / per-unit logic (a port of `analytics.bmu_dispatch`
//! plus `materialize.py`).
//!
//! Definitions (must match `analytics.py` / `materialize.py` exactly):
//! - `pn_mw`: PN level clipped to MEL where MEL is known (MEL is a physical cap).
//! - `boa_level_mw`: winning BOA acceptance (later `acceptance_time` wins on
//!   overlap), clipped to MEL; falls back to `pn_mw` where no acceptance active.
//! - `so_turnup_mw` = max(boa_level - pn, 0)
//! - `boa_curtailment_mw` = max(pn - boa_level, 0)
//! - `so_curtailment_mw` = max(pn - boa_so, 0) with NaN → 0 (only this column
//!   gets the zero-fill, matching `.clip(lower=0).fillna(0)`).
//!
//! Row output rules (must match `materialize._rows_for_units`):
//! - Drop rows where BOTH `pn_mw` and `boa_level_mw` are NaN.
//! - pn/boa/mel NaN → NULL; the three derived columns are raw f64 (NaN stays
//!   NaN in the table, matching psycopg — verify no kept row actually carries
//!   NaN in `so_turnup_mw`/`boa_curtailment_mw`, but keep the semantics anyway).
//! - Columns: `bmu, ts, pn_mw, boa_level_mw, mel_mw, so_turnup_mw, boa_curtailment_mw, so_curtailment_mw`.
//! - `ts` is floored to the freq boundary; grid = `date_range(start, end, freq, inclusive="left")`.
//!
//! Concurrency: split BMUs round-robin across `workers` (default ncpu-1) worker
//! tasks, each with its own `db::connect` connection for fetch + upsert (writes
//! may land out of order — per-chunk keys are disjoint). `freq_minutes` is 5 by
//! default (1 supported).
//!
//! After the upserts, refresh `DISPATCH_CAGGS` over the whole-day window
//! `[start.date 00:00, end.date+1d 00:00)` on a fresh connection.
//!
//! `materialize_dispatch_daily` is the SQL port of `materialize.py`:
//! FULL OUTER JOIN of `bmu_dispatch_daily_cagg` and `b1610_daily_cagg` into
//! `bmu_dispatch_daily` with `ON CONFLICT (bmu, date) DO UPDATE`, MW→MWh via
//! `_DAILY_INTERVAL_HOURS = 5/60`.

use anyhow::Result;
use chrono::{NaiveDate, NaiveDateTime};

use crate::config::Config;

pub const TABLE: &str = "bmu_dispatch";
pub const CONFLICT_COLS: [&str; 2] = ["bmu", "ts"];

/// Continuous aggregates built on bmu_dispatch, refreshed after every
/// materialize (their policies only reach back ~30 days).
pub const DISPATCH_CAGGS: [&str; 3] = [
    "bmu_dispatch_hourly",
    "bmu_dispatch_daily_mw",
    "bmu_dispatch_daily_cagg",
];

pub const DAILY_INTERVAL_HOURS: f64 = 5.0 / 60.0;

pub async fn materialize_dispatch(
    cfg: &Config,
    start: NaiveDateTime,
    end: NaiveDateTime,
    bmus: Option<&[String]>,
    workers: Option<usize>,
    freq_minutes: i64,
) -> Result<usize> {
    todo!("implement materialize_dispatch")
}

pub async fn materialize_dispatch_daily(
    cfg: &Config,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<usize> {
    todo!("implement materialize_dispatch_daily")
}

pub async fn cmd_materialize_dispatch(
    cfg: &Config,
    from: &str,
    to: &str,
    bmus: &[String],
    workers: Option<usize>,
) -> Result<()> {
    todo!("implement cmd_materialize_dispatch")
}

pub async fn cmd_materialize_dispatch_daily(cfg: &Config, from: &str, to: &str) -> Result<()> {
    todo!("implement cmd_materialize_dispatch_daily")
}