use chrono::NaiveDateTime;
use gb_grid_ingest::constraints::{self, parse_rows};
use gb_grid_ingest::db::Value;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

mod common;

const CSV: &str = "\u{feff}Constraint Group,Date (GMT/BST),Limit (MW),Flow (MW)\r\n\
    B6,2026-05-01 00:00:00,1000,900\r\n\
    B6,2026-05-01 00:30:00,1000,\r\n\
    SCOTEX,2026-05-01 00:00:00,2000,1500\r\n";

fn ts(s: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
}

#[test]
fn bom_is_stripped_and_headers_parse() {
    let rows = parse_rows(CSV.as_bytes(), None).unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0], vec![
        Value::text("B6"),
        Value::ts(ts("2026-04-30 23:00:00")),
        Value::f(1000.0),
        Value::f(900.0),
    ]);
    assert_eq!(rows[2][0], Value::text("SCOTEX"));
}

#[test]
fn empty_or_whitespace_float_cells_are_null() {
    let rows = parse_rows(CSV.as_bytes(), None).unwrap();
    assert_eq!(rows[1][2], Value::f(1000.0));
    assert_eq!(rows[1][3], Value::Null);

    let csv = "Constraint Group,Date (GMT/BST),Limit (MW),Flow (MW)\n\
        B6,2026-05-01 00:00:00,   ,\t \n";
    let rows = parse_rows(csv.as_bytes(), None).unwrap();
    assert_eq!(rows[0][2], Value::Null);
    assert_eq!(rows[0][3], Value::Null);
}

#[test]
fn rows_at_or_below_watermark_are_dropped() {
    let rows = parse_rows(CSV.as_bytes(), Some(ts("2026-04-30 23:00:00"))).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0][0], Value::text("B6"));
    assert_eq!(rows[0][1], Value::ts(ts("2026-04-30 23:30:00")));
}

#[test]
fn unparseable_timestamp_rows_are_skipped() {
    let csv = "Constraint Group,Date (GMT/BST),Limit (MW),Flow (MW)\n\
        B6,not-a-date,1000,900\n\
        B6,2026-05-01 00:30:00,1000,900\n";
    let rows = parse_rows(csv.as_bytes(), None).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0][0], Value::text("B6"));
}

#[test]
fn non_numeric_float_cells_skip_the_row() {
    let csv = "Constraint Group,Date (GMT/BST),Limit (MW),Flow (MW)\n\
        B6,2026-05-01 00:00:00,abc,900\n\
        B6,2026-05-01 00:30:00,1000,900\n";
    let rows = parse_rows(csv.as_bytes(), None).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0][1], Value::ts(ts("2026-04-30 23:30:00")));
}

#[test]
fn bst_wall_clock_converts_to_naive_utc() {
    let csv = "Constraint Group,Date (GMT/BST),Limit (MW),Flow (MW)\n\
        B6,2026-06-10 09:30:00,1000,900\n";
    let rows = parse_rows(csv.as_bytes(), None).unwrap();
    assert_eq!(rows[0][1], Value::ts(ts("2026-06-10 08:30:00")));

    let winter = "Constraint Group,Date (GMT/BST),Limit (MW),Flow (MW)\n\
        B6,2026-01-01 12:00:00,1000,900\n";
    let rows = parse_rows(winter.as_bytes(), None).unwrap();
    assert_eq!(rows[0][1], Value::ts(ts("2026-01-01 12:00:00")));
}

/// The header and timestamp format NESO publishes as of Nov 2026.
#[test]
fn current_header_and_iso_t_timestamps_parse() {
    let csv = "\u{feff}Constraint Group,Date_ Time GMT_BST,Limit_MW,Flow_MW\r\n\
        SCOTEX,2026-06-10T09:30:00,5500,6118\r\n\
        SCOTEX,2026-01-14T12:00:00,5400,\r\n";
    let rows = parse_rows(csv.as_bytes(), None).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0][0], Value::text("SCOTEX"));
    assert_eq!(rows[0][1], Value::ts(ts("2026-06-10 08:30:00")));
    assert_eq!(rows[0][2], Value::f(5500.0));
    assert_eq!(rows[0][3], Value::f(6118.0));
    assert_eq!(rows[1][1], Value::ts(ts("2026-01-14 12:00:00")));
    assert_eq!(rows[1][3], Value::Null);
}

/// Regression: ~84k rows of the live file have no seconds (`2023-10-28T00:00`).
/// Python's `fromisoformat` accepts every form here; Rust used to drop them.
#[test]
fn timestamp_forms_match_python_fromisoformat() {
    let csv = "Constraint Group,Date_ Time GMT_BST,Limit_MW,Flow_MW\n\
        A,2026-01-14T12:00,1,1\n\
        B,2026-01-14 12:00,1,1\n\
        C,2026-01-14T12:00:00,1,1\n\
        D,2026-01-14 12:00:00,1,1\n\
        E,2026-01-14T12:00:00.000,1,1\n";
    let rows = parse_rows(csv.as_bytes(), None).unwrap();
    assert_eq!(rows.len(), 5);
    for r in &rows {
        assert_eq!(r[1], Value::ts(ts("2026-01-14 12:00:00")));
    }
}

/// A renamed/missing column must fail loudly rather than skip every row.
#[test]
fn missing_column_is_an_error() {
    let csv = "Constraint Group,When,Limit_MW,Flow_MW\nSCOTEX,2026-06-10T09:30:00,1,2\n";
    let err = parse_rows(csv.as_bytes(), None).unwrap_err().to_string();
    assert!(err.contains("Date_ Time GMT_BST"), "{err}");
}

#[tokio::test]
async fn conditional_get_persists_validators_and_skips_on_304() {
    let Some(cfg) = common::config() else {
        eprintln!("SKIP");
        return;
    };
    let Some(conn) = common::connect().await else {
        eprintln!("SKIP");
        return;
    };
    common::truncate(&conn, &["constraints", "ingest_http_cache"]).await;

    let server = MockServer::start().await;
    let csv_path = reqwest::Url::parse(constraints::CSV_URL)
        .unwrap()
        .path()
        .to_string();
    let url = format!("{}{}", server.uri(), csv_path);

    // First call: no validators in the cache -> plain GET, 200 with new rows.
    // Second call: If-None-Match replayed -> 304 wins (higher priority).
    Mock::given(method("GET"))
        .and(path(csv_path.as_str()))
        .and(header("If-None-Match", "\"v1\""))
        .respond_with(ResponseTemplate::new(304))
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(csv_path.as_str()))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("etag", "\"v1\"")
                .set_body_string(CSV),
        )
        .mount(&server)
        .await;

    let n = constraints::refresh_from_url(&cfg, &url).await.unwrap();
    assert_eq!(n, 3);

    let cnt: i64 = conn
        .query_one("SELECT count(*) FROM constraints", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(cnt, 3);

    let etag: String = conn
        .query_one(
            "SELECT etag FROM ingest_http_cache WHERE url = $1",
            &[&url],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(etag, "\"v1\"");

    // Unchanged file: the server answers 304 and we skip the parse entirely.
    let n = constraints::refresh_from_url(&cfg, &url).await.unwrap();
    assert_eq!(n, 0);

    let cnt: i64 = conn
        .query_one("SELECT count(*) FROM constraints", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(cnt, 3);

    let requests = server.received_requests().await.unwrap();
    let sent = requests.last().unwrap();
    assert_eq!(
        sent.headers
            .get("If-None-Match")
            .and_then(|v| v.to_str().ok()),
        Some("\"v1\"")
    );
}