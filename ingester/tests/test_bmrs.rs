//! Unit tests for the BMRS client: `parse_dt`, `extract_data`, and the
//! per-dataset fetchers against a wiremock HTTP server.

use chrono::{NaiveDate, NaiveDateTime};
use gb_grid_ingest::bmrs::{
    fetch_b1610, fetch_boalf, fetch_fuelinst, fetch_mels, fetch_pn, fetch_system_prices, parse_dt,
    BmrsClient,
};
use gb_grid_ingest::config::{Config, PollConfig};
use gb_grid_ingest::db::Value;
use serde_json::json;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn dt(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(y, mo, d)
        .unwrap()
        .and_hms_opt(h, mi, s)
        .unwrap()
}

fn dt_us(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32, us: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(y, mo, d)
        .unwrap()
        .and_hms_micro_opt(h, mi, s, us)
        .unwrap()
}

fn cfg(base: String) -> Config {
    Config {
        database_url: String::new(),
        api_base: base,
        http_timeout: std::time::Duration::from_secs(60),
        fetch_concurrency: 4,
        poll: PollConfig {
            fuelinst_seconds: 3600,
            boalf_seconds: 3600,
            b1610_seconds: 3600,
            pn_seconds: 3600,
            mels_seconds: 3600,
            system_prices_seconds: 3600,
            materialize_seconds: 3600,
            materialize_daily_seconds: 3600,
        },
    }
}

fn cols(names: &[&str]) -> Vec<String> {
    names.iter().map(|s| s.to_string()).collect()
}

// ---------------------------------------------------------------------------
// parse_dt
// ---------------------------------------------------------------------------

#[test]
fn parse_dt_z_suffix() {
    assert_eq!(parse_dt(&json!("2026-06-10T23:55:00Z")), Some(dt(2026, 6, 10, 23, 55, 0)));
}

#[test]
fn parse_dt_fractional_z() {
    assert_eq!(parse_dt(&json!("2026-06-10T23:55:00.000Z")), Some(dt(2026, 6, 10, 23, 55, 0)));
    assert_eq!(
        parse_dt(&json!("2026-06-10T23:55:00.123456Z")),
        Some(dt_us(2026, 6, 10, 23, 55, 0, 123456))
    );
}

#[test]
fn parse_dt_offset_converts_to_utc() {
    // +01:00 -> 22:55 UTC
    assert_eq!(parse_dt(&json!("2026-06-10T23:55:00+01:00")), Some(dt(2026, 6, 10, 22, 55, 0)));
    // -05:00 -> 04:55 UTC (next day)
    assert_eq!(parse_dt(&json!("2026-06-10T23:55:00-05:00")), Some(dt(2026, 6, 11, 4, 55, 0)));
}

#[test]
fn parse_dt_naive() {
    assert_eq!(parse_dt(&json!("2026-06-10T23:55:00")), Some(dt(2026, 6, 10, 23, 55, 0)));
    // Space separator is also accepted by fromisoformat.
    assert_eq!(parse_dt(&json!("2026-06-10 23:55:00")), Some(dt(2026, 6, 10, 23, 55, 0)));
}

#[test]
fn parse_dt_bare_date_is_midnight() {
    assert_eq!(parse_dt(&json!("2026-06-10")), Some(dt(2026, 6, 10, 0, 0, 0)));
}

#[test]
fn parse_dt_rejects_junk() {
    assert_eq!(parse_dt(&json!(null)), None);
    assert_eq!(parse_dt(&json!("garbage")), None);
    assert_eq!(parse_dt(&json!("2026-06-10T25:99:00Z")), None);
    assert_eq!(parse_dt(&json!(123)), None);
    assert_eq!(parse_dt(&json!(true)), None);
    assert_eq!(parse_dt(&json!({"a": 1})), None);
}

// ---------------------------------------------------------------------------
// extract_data
// ---------------------------------------------------------------------------

#[test]
fn extract_data_top_level_list() {
    let payload = json!([{"a": 1}, {"a": 2}]);
    let out = BmrsClient::extract_data(&payload);
    assert_eq!(out.len(), 2);
    assert_eq!(out[0], &json!({"a": 1}));
}

#[test]
fn extract_data_dict_keys() {
    for (key, expected) in [
        ("data", json!({"a": 1})),
        ("Data", json!({"b": 2})),
        ("results", json!({"c": 3})),
        ("items", json!({"d": 4})),
    ] {
        let mut obj = serde_json::Map::new();
        obj.insert(key.to_string(), json!([expected]));
        obj.insert("other".to_string(), json!([{"z": 9}]));
        let payload = serde_json::Value::Object(obj);
        let out = BmrsClient::extract_data(&payload);
        assert_eq!(out.len(), 1, "key {key}");
        assert_eq!(out[0], &expected, "key {key}");
    }
}

#[test]
fn extract_data_key_order_wins() {
    let payload = json!({"data": [{"a": 1}], "results": [{"b": 2}]});
    let out = BmrsClient::extract_data(&payload);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0], &json!({"a": 1}));
}

#[test]
fn extract_data_empty_and_scalar() {
    assert!(BmrsClient::extract_data(&json!({})).is_empty());
    assert!(BmrsClient::extract_data(&json!(42)).is_empty());
    assert!(BmrsClient::extract_data(&json!(null)).is_empty());
    // "data" present but not a list -> falls through to other keys / empty.
    assert!(BmrsClient::extract_data(&json!({"data": {"a": 1}})).is_empty());
    assert!(BmrsClient::extract_data(&json!({"data": null})).is_empty());
}

// ---------------------------------------------------------------------------
// fetch_* with wiremock
// ---------------------------------------------------------------------------

#[tokio::test]
async fn fetch_fuelinst_maps_spellings_and_skips_bad_rows() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/datasets/FUELINST/stream"))
        .and(query_param("publishDateTimeFrom", "2026-06-10T00:00:00Z"))
        .and(query_param("publishDateTimeTo", "2026-06-10T23:59:59Z"))
        .and(query_param("format", "json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            // All alternative spellings.
            {
                "startTime": "2026-06-10T23:55:00Z",
                "SettlementDate": "2026-06-10",
                "settlementPeriod": 48,
                "psrType": "WIND",
                "quantity": 1234.5,
            },
            // Primary spellings.
            {
                "publishTime": "2026-06-10T22:00:00+01:00",
                "settlementDate": "2026-06-10",
                "settlementPeriod": "47",
                "fuelType": "GAS",
                "generation": 900.25,
            },
            // No publish time -> skipped.
            {
                "SettlementDate": "2026-06-10",
                "fuelType": "OIL",
            },
        ])))
        .mount(&server)
        .await;

    let client = BmrsClient::new(&cfg(server.uri()));
    let (got_cols, rows) = fetch_fuelinst(
        &client,
        dt(2026, 6, 10, 0, 0, 0),
        dt(2026, 6, 10, 23, 59, 59),
    )
    .await
    .unwrap();

    assert_eq!(
        got_cols,
        cols(&["publish_time", "settlement_date", "settlement_period", "fuel_type", "generation_mw"])
    );
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0],
        vec![
            Value::Timestamp(dt(2026, 6, 10, 23, 55, 0)),
            Value::Date(NaiveDate::from_ymd_opt(2026, 6, 10).unwrap()),
            Value::Int(48),
            Value::Text("WIND".to_string()),
            Value::Float(1234.5),
        ]
    );
    // Second row: +01:00 converted to UTC (21:00).
    assert_eq!(
        rows[1],
        vec![
            Value::Timestamp(dt(2026, 6, 10, 21, 0, 0)),
            Value::Date(NaiveDate::from_ymd_opt(2026, 6, 10).unwrap()),
            Value::Int(47),
            Value::Text("GAS".to_string()),
            Value::Float(900.25),
        ]
    );
}

#[tokio::test]
async fn fetch_b1610_sends_full_day_instants() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/datasets/B1610/stream"))
        // The API's from/to are instants; a bare date would only return
        // settlement period 1. Assert the full-day span is sent.
        .and(query_param("from", "2026-06-10T00:00:00Z"))
        .and(query_param("to", "2026-06-10T23:59:59Z"))
        .and(query_param("format", "json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {
                "settlementDate": "2026-06-10",
                "settlementPeriod": 12,
                "bmUnit": "T_DRAX-1",
                "nationalGridBmUnitId": "DRAX-1",
                "quantity": 456.0,
            }
        ])))
        .mount(&server)
        .await;

    let client = BmrsClient::new(&cfg(server.uri()));
    let (got_cols, rows) = fetch_b1610(
        &client,
        NaiveDate::from_ymd_opt(2026, 6, 10).unwrap(),
        NaiveDate::from_ymd_opt(2026, 6, 10).unwrap(),
    )
    .await
    .unwrap();

    assert_eq!(
        got_cols,
        cols(&["settlement_date", "settlement_period", "bm_unit", "ngc_bm_unit", "quantity_mwh"])
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0],
        vec![
            Value::Date(NaiveDate::from_ymd_opt(2026, 6, 10).unwrap()),
            Value::Int(12),
            Value::Text("T_DRAX-1".to_string()),
            Value::Text("DRAX-1".to_string()),
            Value::Float(456.0),
        ]
    );
}

#[tokio::test]
async fn fetch_boalf_bool_flags_as_strings_and_int_acceptance_id() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/datasets/BOALF/stream"))
        .and(query_param("format", "json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {
                "acceptanceNumber": "12345",
                "bmUnit": "T_DRAX-1",
                "nationalGridBmUnit": "DRAX-1",
                "acceptanceTime": "2026-06-10T10:00:00Z",
                "timeFrom": "2026-06-10T10:30:00Z",
                "timeTo": "2026-06-10T11:00:00Z",
                "levelFrom": 50.0,
                "levelTo": 80.0,
                "deemedBoFlag": "true",
                "soFlag": "false",
            },
            // acceptance_id may arrive as a number.
            {
                "acceptanceId": 67890,
                "bmUnit": null,
                "timeFrom": "2026-06-10T12:00:00Z",
                "deemedBoFlag": true,
                "soFlag": "1",
            },
            // Missing acceptance id -> skipped.
            {
                "timeFrom": "2026-06-10T13:00:00Z",
                "bmUnit": "T_SKIP-1",
            }
        ])))
        .mount(&server)
        .await;

    let client = BmrsClient::new(&cfg(server.uri()));
    let (got_cols, rows) = fetch_boalf(
        &client,
        dt(2026, 6, 10, 0, 0, 0),
        dt(2026, 6, 10, 23, 59, 59),
    )
    .await
    .unwrap();

    assert_eq!(
        got_cols,
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
        ])
    );
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0],
        vec![
            Value::Int(12345),
            Value::Text("T_DRAX-1".to_string()),
            Value::Text("DRAX-1".to_string()),
            Value::Timestamp(dt(2026, 6, 10, 10, 0, 0)),
            Value::Timestamp(dt(2026, 6, 10, 10, 30, 0)),
            Value::Timestamp(dt(2026, 6, 10, 11, 0, 0)),
            Value::Float(50.0),
            Value::Float(80.0),
            Value::Bool(true),
            Value::Bool(false),
        ]
    );
    // Null bm_unit stays null; acceptance_id parsed from a number.
    assert_eq!(
        rows[1],
        vec![
            Value::Int(67890),
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Timestamp(dt(2026, 6, 10, 12, 0, 0)),
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Bool(true),
            Value::Bool(true),
        ]
    );
}

#[tokio::test]
async fn fetch_pn_maps_columns() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/datasets/PN/stream"))
        .and(query_param("format", "json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {
                "nationalGridBmUnit": "DRAX-1",
                "bmUnit": "T_DRAX-1",
                "settlementDate": "2026-06-10",
                "settlementPeriod": 24,
                "timeFrom": "2026-06-10T11:30:00Z",
                "timeTo": "2026-06-10T12:00:00Z",
                "levelFrom": 100.0,
                "levelTo": 200.5,
            }
        ])))
        .mount(&server)
        .await;

    let client = BmrsClient::new(&cfg(server.uri()));
    let (got_cols, rows) = fetch_pn(
        &client,
        dt(2026, 6, 10, 0, 0, 0),
        dt(2026, 6, 10, 23, 59, 59),
    )
    .await
    .unwrap();

    assert_eq!(
        got_cols,
        cols(&[
            "national_grid_bm_unit",
            "bm_unit",
            "settlement_date",
            "settlement_period",
            "time_from",
            "time_to",
            "level_from",
            "level_to",
        ])
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0],
        vec![
            Value::Text("DRAX-1".to_string()),
            Value::Text("T_DRAX-1".to_string()),
            Value::Date(NaiveDate::from_ymd_opt(2026, 6, 10).unwrap()),
            Value::Int(24),
            Value::Timestamp(dt(2026, 6, 10, 11, 30, 0)),
            Value::Timestamp(dt(2026, 6, 10, 12, 0, 0)),
            Value::Float(100.0),
            Value::Float(200.5),
        ]
    );
}

#[tokio::test]
async fn fetch_mels_maps_columns() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/datasets/MELS/stream"))
        .and(query_param("format", "json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {
                "nationalGridBmUnit": "DRAX-1",
                "bmUnit": "T_DRAX-1",
                "settlementDate": "2026-06-10",
                "settlementPeriod": 24,
                "timeFrom": "2026-06-10T11:30:00Z",
                "timeTo": "2026-06-10T12:00:00Z",
                "levelFrom": 100.0,
                "levelTo": 200.5,
                "notificationTime": "2026-06-10T11:00:00Z",
                "notificationSequence": "7",
            }
        ])))
        .mount(&server)
        .await;

    let client = BmrsClient::new(&cfg(server.uri()));
    let (got_cols, rows) = fetch_mels(
        &client,
        dt(2026, 6, 10, 0, 0, 0),
        dt(2026, 6, 10, 23, 59, 59),
    )
    .await
    .unwrap();

    assert_eq!(
        got_cols,
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
        ])
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0],
        vec![
            Value::Text("DRAX-1".to_string()),
            Value::Text("T_DRAX-1".to_string()),
            Value::Date(NaiveDate::from_ymd_opt(2026, 6, 10).unwrap()),
            Value::Int(24),
            Value::Timestamp(dt(2026, 6, 10, 11, 30, 0)),
            Value::Timestamp(dt(2026, 6, 10, 12, 0, 0)),
            Value::Float(100.0),
            Value::Float(200.5),
            Value::Timestamp(dt(2026, 6, 10, 11, 0, 0)),
            Value::Int(7),
        ]
    );
}

#[tokio::test]
async fn fetch_system_prices_uses_requested_day() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/balancing/settlement/system-prices/2026-06-10"))
        .and(query_param("format", "json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {
                "settlementPeriod": 1,
                "systemSellPrice": 10.5,
                "systemBuyPrice": -5.2,
                "netImbalanceVolume": 100.0,
            }
        ])))
        .mount(&server)
        .await;

    let client = BmrsClient::new(&cfg(server.uri()));
    let day = NaiveDate::from_ymd_opt(2026, 6, 10).unwrap();
    let (got_cols, rows) = fetch_system_prices(&client, day).await.unwrap();

    assert_eq!(
        got_cols,
        cols(&[
            "settlement_date",
            "settlement_period",
            "system_sell_price",
            "system_buy_price",
            "net_imbalance_volume",
        ])
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0],
        vec![
            Value::Date(day),
            Value::Int(1),
            Value::Float(10.5),
            Value::Float(-5.2),
            Value::Float(100.0),
        ]
    );
}

#[tokio::test]
async fn fetch_system_prices_skips_rows_without_period() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/balancing/settlement/system-prices/2026-06-10"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"settlementPeriod": 1, "systemSellPrice": 10.5},
            {"systemSellPrice": 99.0}
        ])))
        .mount(&server)
        .await;

    let client = BmrsClient::new(&cfg(server.uri()));
    let day = NaiveDate::from_ymd_opt(2026, 6, 10).unwrap();
    let (_, rows) = fetch_system_prices(&client, day).await.unwrap();
    assert_eq!(rows.len(), 1);
}