//! Window-based ingest orchestration and per-dataset ingest entry points.
//!
//! IMPLEMENT THIS MODULE. Do **not** change the public items below — they are
//! consumed by `scheduler.rs` and `main.rs`. You may add private helpers.
//!
//! Port of `src/gb_grid/ingest/base.py`, `fuelinst.py`, `b1610.py`, `boalf.py`,
//! `pn.py`, `mels.py`, `system_prices.py`, and the backfill dispatch in
//! `src/gb_grid/cli.py`.
//!
//! `run_windows` must preserve the Python orchestration semantics exactly:
//! - `concurrency` fetches are in flight at once (default from
//!   `cfg.fetch_concurrency`); the next fetch is submitted *before* the current
//!   window is written (pipelined), so fetching overlaps the serial writes.
//! - Writes stay strictly in window order on the single connection (last-write-
//!   wins and watermark ordering must be unchanged). Use
//!   `futures::stream::iter(windows).map(fetch).buffered(n)` to get exactly
//!   this: n in flight, results yielded in order.
//! - `IngestResult.last_window_with_data` = the last window whose write returned
//!   >0 rows; used by b1610/system_prices for their end-of-day watermarks.
//! - The watermark is advanced to `max(watermark_col)` over the written rows for
//!   datasets that pass `watermark_col`; b1610/system_prices instead advance it
//!   themselves to end-of-day.
//!
//! Chunk sizes per dataset (matching the Python defaults):
//! fuelinst 1d, boalf 6h, pn 12h, mels 12h, b1610 1d, system_prices 1d.

use anyhow::Result;
use chrono::{Duration, NaiveDate, NaiveDateTime};
use futures::Future;
use std::future::ready;
use tokio_postgres::Client;

use crate::bmrs::BmrsClient;
use crate::config::Config;
use crate::db::{self, Row};

pub const DATASETS: [&str; 6] = [
    "fuelinst",
    "b1610",
    "boalf",
    "pn",
    "mels",
    "system_prices",
];

/// Continuous aggregates built on b1610, and the reach of their refresh
/// policies. Anything newer than this is already refreshed automatically every
/// 30 minutes, so refreshing it during ingest too is duplicated work.
pub const B1610_CAGGS: [&str; 2] = ["b1610_daily_cagg", "b1610_hh_cagg"];
pub const CAGG_POLICY_REACH_DAYS: i64 = 30;

#[derive(Debug, Clone)]
pub struct IngestResult<W> {
    pub rows: usize,
    pub last_window_with_data: Option<W>,
}

pub fn daterange(start: NaiveDate, end: NaiveDate, step_days: i64) -> Vec<(NaiveDate, NaiveDate)> {
    todo!("implement daterange")
}

pub fn datetime_range(
    start: NaiveDateTime,
    end: NaiveDateTime,
    step: Duration,
) -> Vec<(NaiveDateTime, NaiveDateTime)> {
    todo!("implement datetime_range")
}

pub async fn run_windows<W, F, Fut>(
    conn: &Client,
    cfg: &Config,
    dataset: &str,
    table: &str,
    conflict_cols: &[&str],
    windows: Vec<W>,
    fetch: F,
    watermark_col: Option<&str>,
    concurrency: Option<usize>,
) -> Result<IngestResult<W>>
where
    W: Clone + Send + Sync + 'static,
    F: Fn(W) -> Fut + Send + Sync,
    Fut: Future<Output = Result<(Vec<String>, Vec<Row>)>> + Send,
{
    todo!("implement run_windows")
}

pub async fn ingest_fuelinst(
    conn: &Client,
    cfg: &Config,
    client: &BmrsClient,
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> Result<usize> {
    todo!("implement ingest_fuelinst")
}

pub async fn ingest_b1610(
    conn: &Client,
    cfg: &Config,
    client: &BmrsClient,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<usize> {
    todo!("implement ingest_b1610 (incl. cagg gating)")
}

pub async fn ingest_boalf(
    conn: &Client,
    cfg: &Config,
    client: &BmrsClient,
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> Result<usize> {
    todo!("implement ingest_boalf")
}

pub async fn ingest_pn(
    conn: &Client,
    cfg: &Config,
    client: &BmrsClient,
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> Result<usize> {
    todo!("implement ingest_pn")
}

pub async fn ingest_mels(
    conn: &Client,
    cfg: &Config,
    client: &BmrsClient,
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> Result<usize> {
    todo!("implement ingest_mels")
}

pub async fn ingest_system_prices(
    conn: &Client,
    cfg: &Config,
    client: &BmrsClient,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<usize> {
    todo!("implement ingest_system_prices")
}

/// CLI `backfill --from --to [--dataset ...]` handler (port of `cli.backfill`).
/// Validates dataset names, loops the selected datasets serially, and prints
/// progress. Returns `Ok(())` when done.
pub async fn cmd_backfill(
    cfg: &Config,
    from: &str,
    to: &str,
    datasets: &[String],
) -> Result<()> {
    todo!("implement cmd_backfill")
}

// Re-export so scheduler.rs can build the fetch closures.
#[allow(dead_code)]
pub fn _ready<T>(v: T) -> impl Future<Output = T> {
    ready(v)
}

// Helper used by run_windows for the watermark max.
#[allow(dead_code)]
pub fn _max_ts<'a>(rows: &'a [Row], col: usize) -> Option<&'a NaiveDateTime> {
    rows.iter()
        .filter_map(|r| match &r[col] {
            db::Value::Timestamp(t) => Some(t),
            _ => None,
        })
        .max()
}