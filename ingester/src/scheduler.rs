//! Always-on scheduler and the remaining CLI handlers.
//!
//! IMPLEMENT THIS MODULE. Do **not** change the public items below — they are
//! consumed by `main.rs`. You may add private helpers.
//!
//! Port of `src/gb_grid/scheduler.py` + the `status`/`refresh-caggs` commands
//! from `src/gb_grid/cli.py`.
//!
//! `run_scheduler` spawns one loop per dataset/pipeline (fuelinst, boalf, b1610,
//! pn, mels, system_prices, materialize_dispatch, materialize_dispatch_daily),
//! each `loop { tick; sleep(interval) }` with errors logged and the loop
//! continuing. Interval from `cfg.poll.<name>_seconds`. The tick windows must
//! match `scheduler.py` verbatim:
//! - fuelinst: watermark (or now−2h) → now
//! - boalf: now−4h → now
//! - pn / mels: now−4h → now+24h
//! - b1610: today−14 → today−3
//! - system_prices: yesterday → today
//! - materialize: now−6h → now (freq 5min)
//! - materialize_daily: today−14 → today
//!
//! Each tick opens a fresh connection and BmrsClient (mirroring the Python
//! `with connect()/BMRSClient()` per tick). Scheduler loops run concurrently
//! (`tokio::spawn` + `join!`).
//!
//! `cmd_status` prints per-table row counts + latest timestamps and the
//! watermarks (port of `cli.status`).
//!
//! `cmd_refresh_caggs` refreshes `DISPATCH_CAGGS` over
//! `[from 00:00, to 23:59:59]` and `B1610_CAGGS` over `[from, to+1d)` (port of
//! `cli.refresh-caggs`).

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use anyhow::Result;
use tokio::task::JoinHandle;

use crate::bmrs::BmrsClient;
use crate::config::Config;
use crate::db;
use crate::{ingest, materialize};

fn now() -> chrono::NaiveDateTime {
    chrono::Utc::now().naive_utc()
}

fn today() -> chrono::NaiveDate {
    chrono::Utc::now().date_naive()
}

/// One tick for the `fuelinst` loop: pull from the watermark (or now−2h) → now.
async fn fuelinst_tick(cfg: &Config) -> Result<()> {
    let conn = db::connect(cfg).await?;
    let client = BmrsClient::new(cfg);
    let now = now();
    let wm = db::get_watermark(&conn, "fuelinst").await?;
    let start = wm.unwrap_or(now - chrono::Duration::hours(2));
    ingest::ingest_fuelinst(&conn, cfg, &client, start, now).await?;
    Ok(())
}

/// Rolling window catches revisions: now−4h → now.
async fn boalf_tick(cfg: &Config) -> Result<()> {
    let conn = db::connect(cfg).await?;
    let client = BmrsClient::new(cfg);
    let now = now();
    let start = now - chrono::Duration::hours(4);
    ingest::ingest_boalf(&conn, cfg, &client, start, now).await?;
    Ok(())
}

/// B1610 has a ~5-working-day publication lag. Sit the window firmly inside the
/// published zone: today−14 → today−3.
async fn b1610_tick(cfg: &Config) -> Result<()> {
    let conn = db::connect(cfg).await?;
    let client = BmrsClient::new(cfg);
    let today = today();
    ingest::ingest_b1610(&conn, cfg, &client, today - chrono::Duration::days(14), today - chrono::Duration::days(3))
        .await?;
    Ok(())
}

async fn pn_tick(cfg: &Config) -> Result<()> {
    let conn = db::connect(cfg).await?;
    let client = BmrsClient::new(cfg);
    let now = now();
    let start = now - chrono::Duration::hours(4);
    ingest::ingest_pn(&conn, cfg, &client, start, now + chrono::Duration::hours(24)).await?;
    Ok(())
}

async fn mels_tick(cfg: &Config) -> Result<()> {
    let conn = db::connect(cfg).await?;
    let client = BmrsClient::new(cfg);
    let now = now();
    let start = now - chrono::Duration::hours(4);
    ingest::ingest_mels(&conn, cfg, &client, start, now + chrono::Duration::hours(24)).await?;
    Ok(())
}

async fn system_prices_tick(cfg: &Config) -> Result<()> {
    let conn = db::connect(cfg).await?;
    let client = BmrsClient::new(cfg);
    let today = today();
    ingest::ingest_system_prices(&conn, cfg, &client, today - chrono::Duration::days(1), today).await?;
    Ok(())
}

async fn materialize_tick(cfg: &Config) -> Result<()> {
    let now = now();
    materialize::materialize_dispatch(cfg, now - chrono::Duration::hours(6), now, None, None, 5).await?;
    Ok(())
}

async fn materialize_daily_tick(cfg: &Config) -> Result<()> {
    // Roll the trailing fortnight forward each cycle. B1610 lags ~5 working days,
    // so older days keep getting refined as actuals trickle in.
    let today = today();
    materialize::materialize_dispatch_daily(cfg, today - chrono::Duration::days(14), today).await?;
    Ok(())
}

/// A tick function boxed so heterogeneous async-fn items can share one loop
/// body. The higher-ranked `Fn(&Config)` keeps the returned future borrowing the
/// config, so the spawned task can own it and re-use it every tick.
type Tick = Box<
    dyn for<'a> Fn(&'a Config) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>
        + Send
        + Sync,
>;

/// Spawn one scheduler loop for a tick function. The task owns a `Config` and
/// loops forever: tick, then sleep, logging (and swallowing) any error so the
/// loop never exits.
fn spawn_loop(name: &'static str, interval_secs: i64, cfg: Config, tick: Tick) -> JoinHandle<()> {
    tokio::spawn(async move {
        let interval = Duration::from_secs(interval_secs.max(1) as u64);
        loop {
            if let Err(e) = tick(&cfg).await {
                crate::log_error!("ingest_failed", "dataset", name, "error", e.to_string());
            }
            tokio::time::sleep(interval).await;
        }
    })
}

/// Box an `async fn(&Config) -> Result<()>`-style tick into a [`Tick`].
fn box_tick<F>(f: F) -> Tick
where
    F: for<'a> Fn(&'a Config) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>
        + Send
        + Sync
        + 'static,
{
    Box::new(f)
}

pub async fn run_scheduler(cfg: Config) -> Result<()> {
    crate::log_info!(
        "scheduler_starting",
        "fuelinst_seconds", cfg.poll.fuelinst_seconds,
        "boalf_seconds", cfg.poll.boalf_seconds,
        "b1610_seconds", cfg.poll.b1610_seconds,
        "pn_seconds", cfg.poll.pn_seconds,
        "mels_seconds", cfg.poll.mels_seconds,
        "system_prices_seconds", cfg.poll.system_prices_seconds,
        "materialize_seconds", cfg.poll.materialize_seconds,
        "materialize_daily_seconds", cfg.poll.materialize_daily_seconds
    );

    let fuelinst = spawn_loop(
        "fuelinst",
        cfg.poll.fuelinst_seconds,
        cfg.clone(),
        box_tick(|c| Box::pin(fuelinst_tick(c))),
    );
    let boalf = spawn_loop(
        "boalf",
        cfg.poll.boalf_seconds,
        cfg.clone(),
        box_tick(|c| Box::pin(boalf_tick(c))),
    );
    let b1610 = spawn_loop(
        "b1610",
        cfg.poll.b1610_seconds,
        cfg.clone(),
        box_tick(|c| Box::pin(b1610_tick(c))),
    );
    let pn = spawn_loop(
        "pn",
        cfg.poll.pn_seconds,
        cfg.clone(),
        box_tick(|c| Box::pin(pn_tick(c))),
    );
    let mels = spawn_loop(
        "mels",
        cfg.poll.mels_seconds,
        cfg.clone(),
        box_tick(|c| Box::pin(mels_tick(c))),
    );
    let system_prices = spawn_loop(
        "system_prices",
        cfg.poll.system_prices_seconds,
        cfg.clone(),
        box_tick(|c| Box::pin(system_prices_tick(c))),
    );
    let materialize_dispatch = spawn_loop(
        "materialize_dispatch",
        cfg.poll.materialize_seconds,
        cfg.clone(),
        box_tick(|c| Box::pin(materialize_tick(c))),
    );
    let materialize_daily = spawn_loop(
        "materialize_dispatch_daily",
        cfg.poll.materialize_daily_seconds,
        cfg,
        box_tick(|c| Box::pin(materialize_daily_tick(c))),
    );

    // Each loop runs forever (errors are logged, never fatal), so this awaits
    // indefinitely — matching `asyncio.gather` in the Python scheduler.
    let _ = tokio::join!(
        fuelinst, boalf, b1610, pn, mels, system_prices, materialize_dispatch, materialize_daily
    );
    Ok(())
}

fn fmt_ts(ts: Option<chrono::NaiveDateTime>) -> String {
    match ts {
        Some(t) => t.to_string(),
        None => "None".to_string(),
    }
}

pub async fn cmd_status(cfg: &Config) -> Result<()> {
    let conn = db::connect(cfg).await?;
    let stats = vec![
        db::table_stats(&conn, "fuelinst", "publish_time").await?,
        db::table_stats(&conn, "b1610", "settlement_date").await?,
        db::table_stats(&conn, "boalf", "time_from").await?,
        db::table_stats(&conn, "pn", "time_from").await?,
        db::table_stats(&conn, "mels", "time_from").await?,
        db::table_stats(&conn, "system_prices", "settlement_date").await?,
    ];
    let wm = conn
        .query(
            "SELECT dataset, last_ts, updated_at FROM ingest_watermark ORDER BY dataset",
            &[],
        )
        .await?;

    println!("db: {}", cfg.database_url);
    println!("{:<14} {:<12} {}", "table", "rows", "latest");
    for s in &stats {
        let latest = s.latest.as_deref().unwrap_or("None");
        println!("{:<14} {:<12} {}", s.table, s.rows, latest);
    }
    println!();
    println!("watermarks:");
    for row in &wm {
        let dataset: String = row.get(0);
        let last_ts: Option<chrono::NaiveDateTime> = row.get(1);
        let updated_at: Option<chrono::NaiveDateTime> = row.get(2);
        println!(
            "  {:<14} last_ts={}  updated_at={}",
            dataset,
            fmt_ts(last_ts),
            fmt_ts(updated_at)
        );
    }
    Ok(())
}

pub async fn cmd_refresh_caggs(cfg: &Config, from: &str, to: &str) -> Result<()> {
    let start_d = crate::config::cli_date(from)?;
    let end_d = crate::config::cli_date(to)?;
    let conn = db::connect(cfg).await?;

    // DISPATCH_CAGGS over `[from 00:00, to 23:59:59.999999]`, matching the
    // Python `datetime.combine(end_d, time.max)`.
    let from_dt = start_d.and_hms_opt(0, 0, 0).expect("00:00:00 is a valid time");
    let to_dt = end_d
        .and_hms_micro_opt(23, 59, 59, 999_999)
        .expect("23:59:59.999999 is a valid time");
    db::refresh_caggs(&conn, &materialize::DISPATCH_CAGGS, from_dt, to_dt).await?;

    // B1610_CAGGS over `[from, to+1d)` — bare dates, matching the Python
    // `refresh_caggs(conn, B1610_CAGGS, start_d, end_d + timedelta(days=1))`.
    db::refresh_caggs_dates(&conn, &ingest::B1610_CAGGS, start_d, end_d + chrono::Duration::days(1))
        .await?;

    println!("refreshed caggs");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;

    /// The eight tick loops must construct and type-check against the fixed
    /// `ingest`/`materialize` signatures. Construct the futures (without polling
    /// them) to prove they compile.
    #[test]
    fn eight_tick_loops_type_check() {
        let cfg = Config {
            database_url: "postgresql://scheduler-test".to_string(),
            api_base: "http://127.0.0.1:1".to_string(),
            http_timeout: Duration::from_secs(1),
            fetch_concurrency: 1,
            poll: crate::config::PollConfig {
                fuelinst_seconds: 1,
                boalf_seconds: 1,
                b1610_seconds: 1,
                pn_seconds: 1,
                mels_seconds: 1,
                system_prices_seconds: 1,
                materialize_seconds: 1,
                materialize_daily_seconds: 1,
            },
        };
        let futures: Vec<Box<dyn Future<Output = Result<()>> + Send + '_>> = vec![
            Box::new(fuelinst_tick(&cfg)),
            Box::new(boalf_tick(&cfg)),
            Box::new(b1610_tick(&cfg)),
            Box::new(pn_tick(&cfg)),
            Box::new(mels_tick(&cfg)),
            Box::new(system_prices_tick(&cfg)),
            Box::new(materialize_tick(&cfg)),
            Box::new(materialize_daily_tick(&cfg)),
        ];
        assert_eq!(futures.len(), 8);
    }
}