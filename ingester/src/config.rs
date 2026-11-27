//! Environment configuration, mirroring `src/gb_grid/config.py`.

use std::env;
use std::time::Duration;

use anyhow::{anyhow, bail, Result};

/// Batch size at which COPY-into-staging overtakes row-wise VALUES upsert.
pub const COPY_MIN_ROWS: usize = 500;

pub const DEFAULT_POLL_SECONDS: i64 = 3600;

pub const POLL_DATASETS: [&str; 8] = [
    "fuelinst",
    "boalf",
    "b1610",
    "pn",
    "mels",
    "system_prices",
    "materialize",
    "materialize_daily",
];

#[derive(Clone)]
pub struct Config {
    pub database_url: String,
    pub api_base: String,
    pub http_timeout: Duration,
    pub fetch_concurrency: usize,
    pub poll: PollConfig,
}

#[derive(Clone)]
pub struct PollConfig {
    pub fuelinst_seconds: i64,
    pub boalf_seconds: i64,
    pub b1610_seconds: i64,
    pub pn_seconds: i64,
    pub mels_seconds: i64,
    pub system_prices_seconds: i64,
    pub materialize_seconds: i64,
    pub materialize_daily_seconds: i64,
}

fn env_or(name: &str, default: &str) -> String {
    env::var(name).unwrap_or_else(|_| default.to_string())
}

fn parse_date(s: &str) -> Result<chrono::NaiveDate> {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|e| anyhow!("invalid date {s:?} (expected YYYY-MM-DD): {e}"))
}

impl Config {
    pub fn from_env() -> Result<Config> {
        let database_url = match env::var("GB_GRID_DATABASE_URL") {
            Ok(v) if !v.trim().is_empty() => v,
            _ => bail!(
                "GB_GRID_DATABASE_URL is not set. \
                 In `nix develop` it's set automatically; otherwise export it."
            ),
        };
        let api_base = env_or("GB_GRID_API_BASE", "https://data.elexon.co.uk/bmrs/api/v1");
        let http_timeout = env::var("GB_GRID_HTTP_TIMEOUT")
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(60.0);
        let fetch_concurrency = env::var("GB_GRID_FETCH_CONCURRENCY")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(4)
            .max(1);
        let poll = PollConfig::from_env()?;
        Ok(Config {
            database_url,
            api_base,
            http_timeout: Duration::from_secs_f64(http_timeout),
            fetch_concurrency,
            poll,
        })
    }
}

impl PollConfig {
    fn poll_seconds(dataset: &str) -> Result<i64> {
        let key = format!("GB_GRID_POLL_{}_SECONDS", dataset.to_uppercase());
        let raw = env::var(&key)
            .or_else(|_| env::var("GB_GRID_POLL_SECONDS"))
            .ok();
        let seconds = match raw {
            Some(s) => s.parse::<i64>().map_err(|_| anyhow!("invalid poll interval for {dataset}: {s}"))?,
            None => DEFAULT_POLL_SECONDS,
        };
        if seconds <= 0 {
            bail!("poll interval for {dataset} must be positive, got {seconds}");
        }
        Ok(seconds)
    }

    pub fn from_env() -> Result<PollConfig> {
        Ok(PollConfig {
            fuelinst_seconds: Self::poll_seconds("fuelinst")?,
            boalf_seconds: Self::poll_seconds("boalf")?,
            b1610_seconds: Self::poll_seconds("b1610")?,
            pn_seconds: Self::poll_seconds("pn")?,
            mels_seconds: Self::poll_seconds("mels")?,
            system_prices_seconds: Self::poll_seconds("system_prices")?,
            materialize_seconds: Self::poll_seconds("materialize")?,
            materialize_daily_seconds: Self::poll_seconds("materialize_daily")?,
        })
    }
}

/// Parse a `YYYY-MM-DD` CLI argument into a date.
pub fn cli_date(s: &str) -> Result<chrono::NaiveDate> {
    parse_date(s)
}