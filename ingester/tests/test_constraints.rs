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
    let rows = parse_rows(CSV.as_bytes(), None);
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
    let rows = parse_rows(CSV.as_bytes(), None);
    assert_eq!(rows[1][2], Value::f(1000.0));
    assert_eq!(rows[1][3], Value::Null);

    let csv = "Constraint Group,Date (GMT/BST),Limit (MW),Flow (MW)\n\
        B6,2026-05-01 00:00:00,   ,\t \n";
    let rows = parse_rows(csv.as_bytes(), None);
    assert_eq!(rows[0][2], Value::Null);
    assert_eq!(rows[0][3], Value::Null);
}

#[test]
fn rows_at_or_below_watermark_are_dropped() {
    let rows = parse_rows(CSV.as_bytes(), Some(ts("2026-04-30 23:00:00")));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0][0], Value::text("B6"));
    assert_eq!(rows[0][1], Value::ts(ts("2026-04-30 23:30:00")));
}

#[test]
fn unparseable_timestamp_rows_are_skipped() {
    let csv = "Constraint Group,Date (GMT/BST),Limit (MW),Flow (MW)\n\
        B6,not-a-date,1000,900\n\
        B6,2026-05-01 00:30:00,1000,900\n";
    let rows = parse_rows(csv.as_bytes(), None);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0][0], Value::text("B6"));
}

#[test]
fn non_numeric_float_cells_skip_the_row() {
    let csv = "Constraint Group,Date (GMT/BST),Limit (MW),Flow (MW)\n\
        B6,2026-05-01 00:00:00,abc,900\n\
        B6,2026-05-01 00:30:00,1000,900\n";
    let rows = parse_rows(csv.as_bytes(), None);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0][1], Value::ts(ts("2026-04-30 23:30:00")));
}

#[test]
fn bst_wall_clock_converts_to_naive_utc() {
    let csv = "Constraint Group,Date (GMT/BST),Limit (MW),Flow (MW)\n\
        B6,2026-06-10 09:30:00,1000,900\n";
    let rows = parse_rows(csv.as_bytes(), None);
    assert_eq!(rows[0][1], Value::ts(ts("2026-06-10 08:30:00")));

    let winter = "Constraint Group,Date (GMT/BST),Limit (MW),Flow (MW)\n\
        B6,2026-01-01 12:00:00,1000,900\n";
    let rows = parse_rows(winter.as_bytes(), None);
    assert_eq!(rows[0][1], Value::ts(ts("2026-01-01 12:00:00")));
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