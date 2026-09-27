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
use chrono::{DateTime, NaiveDate, NaiveDateTime};
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
        if let Some(arr) = payload.as_array() {
            return arr.iter().collect();
        }
        if let Some(obj) = payload.as_object() {
            for k in ["data", "Data", "results", "items"] {
                if let Some(v) = obj.get(k) {
                    if let Some(arr) = v.as_array() {
                        return arr.iter().collect();
                    }
                }
            }
        }
        Vec::new()
    }
}

/// Parse a datetime-ish value. Returns `None` for null / unparseable, mirroring
/// `endpoints._parse_dt` (which replaces `Z` with `+00:00` then calls
/// `fromisoformat`, catching `ValueError`).
///
/// Accepts `Z` / `+HH:MM` offsets / naive ISO8601 strings (with or without
/// fractional seconds, `T` or space separator) and bare dates (midnight).
/// Offsets are converted to UTC and the tzinfo dropped, so the result is always
/// a naive-UTC `NaiveDateTime`.
pub fn parse_dt(v: &Json) -> Option<NaiveDateTime> {
    let raw = v.as_str()?;
    let s: String = match raw.strip_suffix('Z').or_else(|| raw.strip_suffix('z')) {
        Some(rest) => format!("{rest}+00:00"),
        None => raw.to_string(),
    };
    // Timezone-aware strings (RFC3339: requires a colon'd offset).
    if let Ok(dt) = DateTime::parse_from_rfc3339(&s) {
        return Some(dt.naive_utc());
    }
    // Offsets without a colon, e.g. "+0100".
    for fmt in ["%Y-%m-%dT%H:%M:%S%.f%#z", "%Y-%m-%d %H:%M:%S%.f%#z"] {
        if let Ok(dt) = DateTime::parse_from_str(&s, fmt) {
            return Some(dt.naive_utc());
        }
    }
    // Naive datetimes (no offset), optional fractional seconds.
    for fmt in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f"] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(&s, fmt) {
            return Some(dt);
        }
    }
    // A bare date is acceptable (midnight), matching `datetime.fromisoformat`.
    if let Ok(d) = NaiveDate::parse_from_str(&s, "%Y-%m-%d") {
        return d.and_hms_opt(0, 0, 0);
    }
    None
}

/// Format a naive datetime the way the BMRS API expects (`_iso`): naive values
/// are treated as UTC.
fn iso(dt: NaiveDateTime) -> String {
    dt.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

fn cols(names: &[&str]) -> Vec<String> {
    names.iter().map(|s| s.to_string()).collect()
}

/// Coerce a JSON value to `Value::Text` (numbers/bools stringified).
fn to_text(v: &Json) -> Value {
    match v {
        Json::String(s) => Value::Text(s.clone()),
        Json::Number(n) => Value::Text(n.to_string()),
        Json::Bool(b) => Value::Text(b.to_string()),
        _ => Value::Null,
    }
}

/// Coerce a JSON value to `Value::Int`, accepting numbers or numeric strings.
fn to_int(v: &Json) -> Value {
    match v {
        Json::Number(n) => n
            .as_i64()
            .map(Value::Int)
            .or_else(|| n.as_f64().map(|f| Value::Int(f as i64)))
            .unwrap_or(Value::Null),
        Json::String(s) => s
            .parse::<i64>()
            .map(Value::Int)
            .unwrap_or_else(|_| s.parse::<f64>().map(|f| Value::Int(f as i64)).unwrap_or(Value::Null)),
        _ => Value::Null,
    }
}

/// Coerce a JSON value to `Value::Float`, accepting numbers or numeric strings.
fn to_float(v: &Json) -> Value {
    match v {
        Json::Number(n) => n.as_f64().map(Value::Float).unwrap_or(Value::Null),
        Json::String(s) => s.parse::<f64>().map(Value::Float).unwrap_or(Value::Null),
        _ => Value::Null,
    }
}

/// Coerce a JSON value to `Value::Bool`, accepting bools or
/// "true"/"false" strings (lenient).
fn to_bool(v: &Json) -> Value {
    match v {
        Json::Bool(b) => Value::Bool(*b),
        Json::String(s) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "t" | "yes" => Value::Bool(true),
            "false" | "0" | "f" | "no" => Value::Bool(false),
            _ => Value::Null,
        },
        _ => Value::Null,
    }
}

/// Parse a JSON value into a `Value::Date` (the date part of the parsed stamp).
fn to_date(v: &Json) -> Value {
    match parse_dt(v) {
        Some(dt) => Value::Date(dt.date()),
        None => Value::Null,
    }
}

/// Parse a JSON value into a `Value::Timestamp` (naive UTC).
fn to_ts(v: &Json) -> Value {
    match parse_dt(v) {
        Some(dt) => Value::Timestamp(dt),
        None => Value::Null,
    }
}

pub async fn fetch_fuelinst(
    client: &BmrsClient,
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> Result<(Vec<String>, Vec<Row>)> {
    let payload = client
        .get(
            "/datasets/FUELINST/stream",
            &[
                ("publishDateTimeFrom", iso(start)),
                ("publishDateTimeTo", iso(end)),
                ("format", "json".to_string()),
            ],
        )
        .await?;
    let mut rows: Vec<Row> = Vec::new();
    for r in BmrsClient::extract_data(&payload) {
        let publish = match get_field(r, &["publishTime", "PublishTime", "startTime", "StartTime"])
            .and_then(parse_dt)
        {
            Some(p) => p,
            None => continue,
        };
        rows.push(vec![
            Value::Timestamp(publish),
            get_field(r, &["settlementDate", "SettlementDate"])
                .map(to_date)
                .unwrap_or(Value::Null),
            get_field(r, &["settlementPeriod", "SettlementPeriod"])
                .map(to_int)
                .unwrap_or(Value::Null),
            get_field(r, &["fuelType", "FuelType", "psrType"])
                .map(to_text)
                .unwrap_or(Value::Null),
            get_field(r, &["generation", "Generation", "quantity"])
                .map(to_float)
                .unwrap_or(Value::Null),
        ]);
    }
    Ok((
        cols(&[
            "publish_time",
            "settlement_date",
            "settlement_period",
            "fuel_type",
            "generation_mw",
        ]),
        rows,
    ))
}

pub async fn fetch_b1610(
    client: &BmrsClient,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<(Vec<String>, Vec<Row>)> {
    // The API's from/to are instants; a bare date only returns settlement
    // period 1, so send the full-day datetime span.
    let start_dt = start.and_hms_opt(0, 0, 0).expect("valid midnight");
    let end_dt = end.and_hms_opt(23, 59, 59).expect("valid 23:59:59");
    let payload = client
        .get(
            "/datasets/B1610/stream",
            &[
                ("from", iso(start_dt)),
                ("to", iso(end_dt)),
                ("format", "json".to_string()),
            ],
        )
        .await?;
    let mut rows: Vec<Row> = Vec::new();
    for r in BmrsClient::extract_data(&payload) {
        let sd = get_field(r, &["settlementDate", "SettlementDate"]).and_then(parse_dt);
        let bm_unit = get_field(r, &["bmUnit", "BmUnit", "bMUnitId"]);
        let period = get_field(r, &["settlementPeriod", "SettlementPeriod"]);
        if sd.is_none() || bm_unit.is_none() || period.is_none() {
            continue;
        }
        rows.push(vec![
            Value::Date(sd.expect("checked above").date()),
            to_int(period.expect("checked above")),
            to_text(bm_unit.expect("checked above")),
            get_field(r, &["nationalGridBmUnitId", "nationalGridBmUnit", "ngcBmUnit"])
                .map(to_text)
                .unwrap_or(Value::Null),
            get_field(r, &["quantity", "Quantity", "generation"])
                .map(to_float)
                .unwrap_or(Value::Null),
        ]);
    }
    Ok((
        cols(&[
            "settlement_date",
            "settlement_period",
            "bm_unit",
            "ngc_bm_unit",
            "quantity_mwh",
        ]),
        rows,
    ))
}

pub async fn fetch_boalf(
    client: &BmrsClient,
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> Result<(Vec<String>, Vec<Row>)> {
    let payload = client
        .get(
            "/datasets/BOALF/stream",
            &[
                ("from", iso(start)),
                ("to", iso(end)),
                ("format", "json".to_string()),
            ],
        )
        .await?;
    let mut rows: Vec<Row> = Vec::new();
    for r in BmrsClient::extract_data(&payload) {
        let time_from = get_field(r, &["timeFrom", "TimeFrom"]).and_then(parse_dt);
        let accept_id = get_field(r, &["acceptanceNumber", "AcceptanceNumber", "acceptanceId"]);
        if time_from.is_none() || accept_id.is_none() {
            continue;
        }
        rows.push(vec![
            to_int(accept_id.expect("checked above")),
            get_field(r, &["bmUnit", "BmUnit"]).map(to_text).unwrap_or(Value::Null),
            get_field(r, &["nationalGridBmUnit", "NationalGridBmUnit"])
                .map(to_text)
                .unwrap_or(Value::Null),
            get_field(r, &["acceptanceTime", "AcceptanceTime"])
                .and_then(parse_dt)
                .map(Value::Timestamp)
                .unwrap_or(Value::Null),
            Value::Timestamp(time_from.expect("checked above")),
            get_field(r, &["timeTo", "TimeTo"])
                .and_then(parse_dt)
                .map(Value::Timestamp)
                .unwrap_or(Value::Null),
            get_field(r, &["levelFrom", "LevelFrom"]).map(to_float).unwrap_or(Value::Null),
            get_field(r, &["levelTo", "LevelTo"]).map(to_float).unwrap_or(Value::Null),
            get_field(r, &["deemedBoFlag", "DeemedBoFlag"]).map(to_bool).unwrap_or(Value::Null),
            get_field(r, &["soFlag", "SoFlag"]).map(to_bool).unwrap_or(Value::Null),
        ]);
    }
    Ok((
        cols(&[
            "acceptance_id",
            "bm_unit",
            "ngc_bm_unit",
            "acceptance_time",
            "time_from",
            "time_to",
            "level_from",
            "level_to",
            "deemed_bo_flag",
            "so_flag",
        ]),
        rows,
    ))
}

pub async fn fetch_pn(
    client: &BmrsClient,
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> Result<(Vec<String>, Vec<Row>)> {
    let payload = client
        .get(
            "/datasets/PN/stream",
            &[
                ("from", iso(start)),
                ("to", iso(end)),
                ("format", "json".to_string()),
            ],
        )
        .await?;
    let mut rows: Vec<Row> = Vec::new();
    for r in BmrsClient::extract_data(&payload) {
        let ngc = get_field(r, &["nationalGridBmUnit", "NationalGridBmUnit"]);
        let time_from = get_field(r, &["timeFrom", "TimeFrom"]).and_then(parse_dt);
        if ngc.is_none() || time_from.is_none() {
            continue;
        }
        let sd = get_field(r, &["settlementDate", "SettlementDate"]).and_then(parse_dt);
        rows.push(vec![
            to_text(ngc.expect("checked above")),
            get_field(r, &["bmUnit", "BmUnit"]).map(to_text).unwrap_or(Value::Null),
            sd.map(|d| Value::Date(d.date())).unwrap_or(Value::Null),
            get_field(r, &["settlementPeriod", "SettlementPeriod"])
                .map(to_int)
                .unwrap_or(Value::Null),
            Value::Timestamp(time_from.expect("checked above")),
            get_field(r, &["timeTo", "TimeTo"])
                .and_then(parse_dt)
                .map(Value::Timestamp)
                .unwrap_or(Value::Null),
            get_field(r, &["levelFrom", "LevelFrom"]).map(to_float).unwrap_or(Value::Null),
            get_field(r, &["levelTo", "LevelTo"]).map(to_float).unwrap_or(Value::Null),
        ]);
    }
    Ok((
        cols(&[
            "national_grid_bm_unit",
            "bm_unit",
            "settlement_date",
            "settlement_period",
            "time_from",
            "time_to",
            "level_from",
            "level_to",
        ]),
        rows,
    ))
}

pub async fn fetch_mels(
    client: &BmrsClient,
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> Result<(Vec<String>, Vec<Row>)> {
    let payload = client
        .get(
            "/datasets/MELS/stream",
            &[
                ("from", iso(start)),
                ("to", iso(end)),
                ("format", "json".to_string()),
            ],
        )
        .await?;
    let mut rows: Vec<Row> = Vec::new();
    for r in BmrsClient::extract_data(&payload) {
        let ngc = get_field(r, &["nationalGridBmUnit", "NationalGridBmUnit"]);
        let time_from = get_field(r, &["timeFrom", "TimeFrom"]).and_then(parse_dt);
        let seq = get_field(r, &["notificationSequence", "NotificationSequence"]);
        if ngc.is_none() || time_from.is_none() || seq.is_none() {
            continue;
        }
        let sd = get_field(r, &["settlementDate", "SettlementDate"]).and_then(parse_dt);
        rows.push(vec![
            to_text(ngc.expect("checked above")),
            get_field(r, &["bmUnit", "BmUnit"]).map(to_text).unwrap_or(Value::Null),
            sd.map(|d| Value::Date(d.date())).unwrap_or(Value::Null),
            get_field(r, &["settlementPeriod", "SettlementPeriod"])
                .map(to_int)
                .unwrap_or(Value::Null),
            Value::Timestamp(time_from.expect("checked above")),
            get_field(r, &["timeTo", "TimeTo"])
                .and_then(parse_dt)
                .map(Value::Timestamp)
                .unwrap_or(Value::Null),
            get_field(r, &["levelFrom", "LevelFrom"]).map(to_float).unwrap_or(Value::Null),
            get_field(r, &["levelTo", "LevelTo"]).map(to_float).unwrap_or(Value::Null),
            get_field(r, &["notificationTime", "NotificationTime"])
                .and_then(parse_dt)
                .map(Value::Timestamp)
                .unwrap_or(Value::Null),
            to_int(seq.expect("checked above")),
        ]);
    }
    Ok((
        cols(&[
            "national_grid_bm_unit",
            "bm_unit",
            "settlement_date",
            "settlement_period",
            "time_from",
            "time_to",
            "level_from",
            "level_to",
            "notification_time",
            "notification_sequence",
        ]),
        rows,
    ))
}

pub async fn fetch_system_prices(
    client: &BmrsClient,
    day: NaiveDate,
) -> Result<(Vec<String>, Vec<Row>)> {
    let path = format!("/balancing/settlement/system-prices/{}", day.format("%Y-%m-%d"));
    let payload = client.get(&path, &[("format", "json".to_string())]).await?;
    let mut rows: Vec<Row> = Vec::new();
    for r in BmrsClient::extract_data(&payload) {
        let period = get_field(r, &["settlementPeriod", "SettlementPeriod"]);
        if period.is_none() {
            continue;
        }
        rows.push(vec![
            Value::Date(day),
            to_int(period.expect("checked above")),
            get_field(r, &["systemSellPrice", "SystemSellPrice"])
                .map(to_float)
                .unwrap_or(Value::Null),
            get_field(r, &["systemBuyPrice", "SystemBuyPrice"])
                .map(to_float)
                .unwrap_or(Value::Null),
            get_field(r, &["netImbalanceVolume", "NetImbalanceVolume"])
                .map(to_float)
                .unwrap_or(Value::Null),
        ]);
    }
    Ok((
        cols(&[
            "settlement_date",
            "settlement_period",
            "system_sell_price",
            "system_buy_price",
            "net_imbalance_volume",
        ]),
        rows,
    ))
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