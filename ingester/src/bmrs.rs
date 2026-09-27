//! BMRS Insights API client and per-dataset fetchers.
//!
//! IMPLEMENT THIS MODULE. Do **not** change the public items below — they are
//! consumed by `ingest.rs`, `scheduler.rs`, and `main.rs`. You may add private
//! helpers as needed.
//!
//! Port of `src/gb_grid/api/client.py` + `src/gb_grid/api/endpoints.py`.
//!
//! Key semantics to preserve:
//! - `extract_data`: payload is a top-level list, or a dict carrying a list
//!   under `data` / `Data` / `results` / `items`.
//! - Tolerant key extraction (`_get`): each field has multiple known spellings
//!   across the API's responses (e.g. `publishTime`/`PublishTime`/`startTime`).
//! - `_parse_dt`: accept `Z` / offsets / naive ISO8601 strings; unparseable
//!   values → skip the row (`None`).
//! - Retry is handled by `crate::http::Http::get`; use that, not raw reqwest.
//! - Timestamps are naive UTC (`NaiveDateTime`); dates are `NaiveDate`.
//!
//! Column lists (order matters — they become both the INSERT columns and the
//! COPY columns) must match the table schemas in `0001.init.sql`:
//! - fuelinst: `publish_time, settlement_date, settlement_period, fuel_type, generation_mw`
//! - b1610: `settlement_date, settlement_period, bm_unit, ngc_bm_unit, quantity_mwh`
//! - boalf: `acceptance_id, bm_unit, ngc_bm_unit, acceptance_time, time_from, time_to, level_from, level_to, deemed_bo_flag, so_flag`
//! - pn: `national_grid_bm_unit, bm_unit, settlement_date, settlement_period, time_from, time_to, level_from, level_to`
//! - mels: `national_grid_bm_unit, bm_unit, settlement_date, settlement_period, time_from, time_to, level_from, level_to, notification_time, notification_sequence`
//! - system_prices: `settlement_date, settlement_period, system_sell_price, system_buy_price, net_imbalance_volume`

use anyhow::Result;
use chrono::{NaiveDate, NaiveDateTime};
use serde_json::Value as Json;

use crate::config::Config;
use crate::db::{Row, Value};

pub struct BmrsClient {
    http: crate::http::Http,
}

impl BmrsClient {
    pub fn new(cfg: &Config) -> BmrsClient {
        BmrsClient {
            http: crate::http::Http::new(cfg),
        }
    }

    /// GET a dataset endpoint (with retry) and return the raw JSON payload.
    pub async fn get(&self, path: &str, params: &[(&str, String)]) -> Result<Json> {
        self.http.get(path, params).await
    }

    /// Normalise a BMRS payload to a list of records.
    pub fn extract_data(payload: &Json) -> Vec<&Json> {
        todo!("implement extract_data")
    }
}

/// Parse a datetime-ish value. Returns `None` for null / unparseable, mirroring
/// `endpoints._parse_dt` (which replaces `Z` with `+00:00` then calls
/// `fromisoformat`, catching `ValueError`).
pub fn parse_dt(v: &Json) -> Option<NaiveDateTime> {
    todo!("implement parse_dt")
}

pub async fn fetch_fuelinst(
    client: &BmrsClient,
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> Result<(Vec<String>, Vec<Row>)> {
    todo!("implement fetch_fuelinst")
}

pub async fn fetch_b1610(
    client: &BmrsClient,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<(Vec<String>, Vec<Row>)> {
    todo!("implement fetch_b1610")
}

pub async fn fetch_boalf(
    client: &BmrsClient,
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> Result<(Vec<String>, Vec<Row>)> {
    todo!("implement fetch_boalf")
}

pub async fn fetch_pn(
    client: &BmrsClient,
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> Result<(Vec<String>, Vec<Row>)> {
    todo!("implement fetch_pn")
}

pub async fn fetch_mels(
    client: &BmrsClient,
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> Result<(Vec<String>, Vec<Row>)> {
    todo!("implement fetch_mels")
}

pub async fn fetch_system_prices(
    client: &BmrsClient,
    day: NaiveDate,
) -> Result<(Vec<String>, Vec<Row>)> {
    todo!("implement fetch_system_prices")
}

// Convenience: helper for the tolerant `_get`.
#[allow(dead_code)]
fn get_field<'a>(record: &'a Json, keys: &[&str]) -> Option<&'a Json> {
    let obj = record.as_object()?;
    for k in keys {
        if let Some(v) = obj.get(*k) {
            if !v.is_null() {
                return Some(v);
            }
        }
    }
    None
}