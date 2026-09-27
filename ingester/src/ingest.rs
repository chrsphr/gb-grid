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

use anyhow::{bail, Result};
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

/// Inclusive date windows: `(start..end)` stepped by `step_days`, yielding
/// `(cur, min(cur + step - 1 day, end))`.
pub fn daterange(start: NaiveDate, end: NaiveDate, step_days: i64) -> Vec<(NaiveDate, NaiveDate)> {
    let mut out = Vec::new();
    let mut cur = start;
    while cur <= end {
        let nxt = (cur + Duration::days(step_days)).min(end + Duration::days(1));
        out.push((cur, (nxt - Duration::days(1)).min(end)));
        cur = nxt;
    }
    out
}

/// Half-open datetime windows: `(cur, min(cur + step, end))` while `cur < end`.
pub fn datetime_range(
    start: NaiveDateTime,
    end: NaiveDateTime,
    step: Duration,
) -> Vec<(NaiveDateTime, NaiveDateTime)> {
    let mut out = Vec::new();
    let mut cur = start;
    while cur < end {
        let nxt = (cur + step).min(end);
        out.push((cur, nxt));
        cur = nxt;
    }
    out
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
    use futures::StreamExt;

    let n = concurrency
        .map(|c| c.max(1))
        .unwrap_or(cfg.fetch_concurrency)
        .max(1);

    // `buffered(n)` keeps n fetches in flight and yields their results in
    // window order; the next fetch is submitted when the previous one is
    // consumed, so fetching overlaps the serial writes below.
    let mut stream = Box::pin(
        futures::stream::iter(windows)
            .map(move |w| {
                let fut = fetch(w.clone());
                Box::pin(async move {
                    let (cols, rows) = fut.await?;
                    Ok::<_, anyhow::Error>((w, cols, rows))
                })
            })
            .buffered(n),
    );

    let mut total = 0usize;
    let mut last_with_data: Option<W> = None;
    while let Some(result) = stream.next().await {
        let (w, cols, rows) = result?;
        let written = db::upsert(conn, table, &cols, &rows, conflict_cols).await?;
        total += written;
        if written > 0 {
            last_with_data = Some(w);
        }
        if let Some(wcol) = watermark_col {
            if !rows.is_empty() {
                if let Some(col) = cols.iter().position(|c| c == wcol) {
                    if let Some(ts) = _max_ts(&rows, col) {
                        db::set_watermark(conn, dataset, *ts).await?;
                    }
                }
            }
        }
        crate::log_info!("ingested", "dataset", dataset, "rows", written);
    }
    Ok(IngestResult {
        rows: total,
        last_window_with_data: last_with_data,
    })
}

pub async fn ingest_fuelinst(
    conn: &Client,
    cfg: &Config,
    client: &BmrsClient,
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> Result<usize> {
    let windows = datetime_range(start, end, Duration::days(1));
    let fetch = move |w: (NaiveDateTime, NaiveDateTime)| {
        let (s, e) = w;
        async move { crate::bmrs::fetch_fuelinst(client, s, e).await }
    };
    let res = run_windows(
        conn,
        cfg,
        "fuelinst",
        "fuelinst",
        &["publish_time", "fuel_type"],
        windows,
        fetch,
        Some("publish_time"),
        None,
    )
    .await?;
    Ok(res.rows)
}

pub async fn ingest_b1610(
    conn: &Client,
    cfg: &Config,
    client: &BmrsClient,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<usize> {
    let windows = daterange(start, end, 1);
    let fetch = move |w: (NaiveDate, NaiveDate)| {
        let (s, e) = w;
        async move { crate::bmrs::fetch_b1610(client, s, e).await }
    };
    let res = run_windows(
        conn,
        cfg,
        "b1610",
        "b1610",
        &["settlement_date", "settlement_period", "bm_unit"],
        windows,
        fetch,
        None,
        None,
    )
    .await?;
    if let Some((_day0, day1)) = &res.last_window_with_data {
        // B1610 lacks a single timestamp column; advance watermark to end of
        // the last covered day.
        let last_ts = day1
            .and_hms_micro_opt(23, 59, 59, 999_999)
            .expect("23:59:59.999999 is valid");
        db::set_watermark(conn, "b1610", last_ts).await?;
        // Fold the loaded rows into the caggs the dashboards read, but only for
        // the part of the window the refresh policies don't already cover.
        // The b1610 caggs bucket settlement_date (a DATE), so pass date args.
        let policy_floor = chrono::Utc::now().date_naive() - Duration::days(CAGG_POLICY_REACH_DAYS);
        if start < policy_floor {
            let refresh_end = end.min(policy_floor) + Duration::days(1);
            db::refresh_caggs_dates(conn, &B1610_CAGGS, start, refresh_end).await?;
        }
    }
    Ok(res.rows)
}

pub async fn ingest_boalf(
    conn: &Client,
    cfg: &Config,
    client: &BmrsClient,
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> Result<usize> {
    let windows = datetime_range(start, end, Duration::hours(6));
    let fetch = move |w: (NaiveDateTime, NaiveDateTime)| {
        let (s, e) = w;
        async move { crate::bmrs::fetch_boalf(client, s, e).await }
    };
    let res = run_windows(
        conn,
        cfg,
        "boalf",
        "boalf",
        &["acceptance_id", "time_from"],
        windows,
        fetch,
        Some("time_from"),
        None,
    )
    .await?;
    Ok(res.rows)
}

pub async fn ingest_pn(
    conn: &Client,
    cfg: &Config,
    client: &BmrsClient,
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> Result<usize> {
    let windows = datetime_range(start, end, Duration::hours(12));
    let fetch = move |w: (NaiveDateTime, NaiveDateTime)| {
        let (s, e) = w;
        async move { crate::bmrs::fetch_pn(client, s, e).await }
    };
    let res = run_windows(
        conn,
        cfg,
        "pn",
        "pn",
        &["national_grid_bm_unit", "time_from"],
        windows,
        fetch,
        Some("time_from"),
        None,
    )
    .await?;
    Ok(res.rows)
}

pub async fn ingest_mels(
    conn: &Client,
    cfg: &Config,
    client: &BmrsClient,
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> Result<usize> {
    let windows = datetime_range(start, end, Duration::hours(12));
    let fetch = move |w: (NaiveDateTime, NaiveDateTime)| {
        let (s, e) = w;
        async move { crate::bmrs::fetch_mels(client, s, e).await }
    };
    let res = run_windows(
        conn,
        cfg,
        "mels",
        "mels",
        &["national_grid_bm_unit", "time_from", "notification_sequence"],
        windows,
        fetch,
        Some("time_from"),
        None,
    )
    .await?;
    Ok(res.rows)
}

pub async fn ingest_system_prices(
    conn: &Client,
    cfg: &Config,
    client: &BmrsClient,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<usize> {
    let mut days = Vec::new();
    let mut d = start;
    while d <= end {
        days.push(d);
        d = d + Duration::days(1);
    }
    let fetch = move |day: NaiveDate| async move { crate::bmrs::fetch_system_prices(client, day).await };
    let res = run_windows(
        conn,
        cfg,
        "system_prices",
        "system_prices",
        &["settlement_date", "settlement_period"],
        days,
        fetch,
        None,
        None,
    )
    .await?;
    if let Some(day) = &res.last_window_with_data {
        let last_ts = day
            .and_hms_micro_opt(23, 59, 59, 999_999)
            .expect("23:59:59.999999 is valid");
        db::set_watermark(conn, "system_prices", last_ts).await?;
    }
    Ok(res.rows)
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
    let start_d = crate::config::cli_date(from)?;
    let end_d = crate::config::cli_date(to)?;

    let selected: Vec<&str> = if datasets.is_empty() {
        DATASETS.to_vec()
    } else {
        let unknown: Vec<&String> = datasets
            .iter()
            .filter(|d| !DATASETS.contains(&d.as_str()))
            .collect();
        if !unknown.is_empty() {
            bail!("unknown datasets: {unknown:?}");
        }
        datasets.iter().map(|s| s.as_str()).collect()
    };

    let start_dt = start_d.and_hms_opt(0, 0, 0).expect("midnight is valid");
    let end_dt = end_d
        .and_hms_micro_opt(23, 59, 59, 999_999)
        .expect("23:59:59.999999 is valid");

    let conn = db::connect(cfg).await?;
    let client = BmrsClient::new(cfg);
    for d in selected {
        println!("-> {d}: {start_d} .. {end_d}");
        match d {
            "fuelinst" => ingest_fuelinst(&conn, cfg, &client, start_dt, end_dt).await?,
            "b1610" => ingest_b1610(&conn, cfg, &client, start_d, end_d).await?,
            "boalf" => ingest_boalf(&conn, cfg, &client, start_dt, end_dt).await?,
            "pn" => ingest_pn(&conn, cfg, &client, start_dt, end_dt).await?,
            "mels" => ingest_mels(&conn, cfg, &client, start_dt, end_dt).await?,
            "system_prices" => ingest_system_prices(&conn, cfg, &client, start_d, end_d).await?,
            _ => bail!("unknown dataset {d}"),
        };
    }
    Ok(())
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