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

use anyhow::Result;

use crate::config::Config;

pub async fn run_scheduler(cfg: Config) -> Result<()> {
    todo!("implement run_scheduler")
}

pub async fn cmd_status(cfg: &Config) -> Result<()> {
    todo!("implement cmd_status")
}

pub async fn cmd_refresh_caggs(cfg: &Config, from: &str, to: &str) -> Result<()> {
    todo!("implement cmd_refresh_caggs")
}