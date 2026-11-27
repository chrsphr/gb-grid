//! NESO day-ahead constraint flows: streaming CSV refresh with conditional GET.
//!
//! Port of `src/gb_grid/ingest/constraints.py`.
//!
//! Key semantics preserved:
//! - `CSV_URL` is the fixed NESO resource URL (follows redirects).
//! - Send `If-None-Match` / `If-Modified-Since` validators read from the
//!   `ingest_http_cache` table; a 304 means unchanged → return 0 rows and do
//!   **not** touch the cached validators.
//! - Drop the UTF-8 BOM from the first line so the header parses.
//! - Parse the `Date_ Time GMT_BST` (formerly `Date (GMT/BST)`) column as Europe/London wall-clock → naive UTC, memoised on
//!   the raw string (each timestamp repeats across every constraint group).
//! - Filter `ts <= last_ts` (the high-water mark = `MAX(ts)` from the table)
//!   *before* building rows — on a daily refresh nearly all rows are dropped.
//! - `Limit_MW` / `Flow_MW` → float-or-null; skip rows that fail to parse. A
//!   missing column is an error (it used to skip every row silently).
//! - On success, persist the new validators back into `ingest_http_cache`.
//! - Upsert via `db::upsert` with table `constraints`, conflict `constraint_group, ts`.
//!
//! The download itself has **no** retry (matching the Python httpx path); it uses
//! `crate::http::Http::stream` with a 120s timeout. The response body is buffered
//! in memory (`resp.bytes()`) rather than streamed line by line — acceptable for
//! this port since the watermark filter still runs before any row is built.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use chrono::{LocalResult, NaiveDateTime, TimeZone};
use chrono_tz::Europe::London;
use csv::ReaderBuilder;

use crate::config::Config;
use crate::db::{self, Row, Value};
use crate::http::Http;

pub const CSV_URL: &str = "https://api.neso.energy/dataset/cf3cbc92-2d5d-4c2b-bd29-e11a21070b26/resource/38a18ec1-9e40-465d-93fb-301e80fd1352/download/day-ahead-constraints-limits-and-flow-output-v1.5.csv";

pub const TABLE: &str = "constraints";
pub const CONFLICT: [&str; 2] = ["constraint_group", "ts"];

// CSV column names, current spelling first. As of Nov 2026 NESO publishes
// `Date_ Time GMT_BST` / `Limit_MW` / `Flow_MW` with ISO `T` timestamps; older
// copies used the parenthesised names. A missing column is a hard error. Skipping
// every row instead is how the rename went unnoticed (0 rows, no failure).
const HDR_GROUP: &[&str] = &["Constraint Group"];
const HDR_DATE: &[&str] = &["Date_ Time GMT_BST", "Date (GMT/BST)"];
const HDR_LIMIT: &[&str] = &["Limit_MW", "Limit (MW)"];
const HDR_FLOW: &[&str] = &["Flow_MW", "Flow (MW)"];

const FETCH_TIMEOUT: Duration = Duration::from_secs(120);

/// CLI `refresh-constraints` handler. Downloads the CSV, upserts new rows, and
/// returns the number of rows written (0 for an unchanged file).
pub async fn cmd_refresh_constraints(cfg: &Config) -> Result<usize> {
    refresh_from_url(cfg, CSV_URL).await
}

/// Full conditional-GET refresh loop against an injectable source URL.
///
/// The CLI path always targets [`CSV_URL`]; tests point this at a mock server.
pub async fn refresh_from_url(cfg: &Config, url: &str) -> Result<usize> {
    let conn = db::connect(cfg).await.context("connect for constraints refresh")?;

    let last_ts: Option<NaiveDateTime> = conn
        .query_opt("SELECT MAX(ts) FROM constraints", &[])
        .await
        .context("read constraints high-water mark")?
        .and_then(|r| r.get::<_, Option<NaiveDateTime>>(0));
    let last_ts_str = last_ts
        .map(|t| t.to_string())
        .unwrap_or_else(|| "None".to_string());
    crate::log_info!("constraints_fetch_start", "last_ts", last_ts_str);

    let (etag, last_modified): (Option<String>, Option<String>) = conn
        .query_opt(
            "SELECT etag, last_modified FROM ingest_http_cache WHERE url = $1",
            &[&url],
        )
        .await
        .context("read cached validators")?
        .map(|r| (r.get(0), r.get(1)))
        .unwrap_or((None, None));

    let mut headers: Vec<(&str, &str)> = Vec::with_capacity(2);
    if let Some(etag) = etag.as_deref() {
        headers.push(("If-None-Match", etag));
    }
    if let Some(last_modified) = last_modified.as_deref() {
        headers.push(("If-Modified-Since", last_modified));
    }

    let http = Http::new(cfg);
    let resp = http.stream(url, &headers, FETCH_TIMEOUT).await?;

    if resp.status() == reqwest::StatusCode::NOT_MODIFIED {
        crate::log_info!("constraints_not_modified");
        return Ok(0);
    }
    resp.error_for_status_ref().context("constraints download")?;

    let new_etag = resp
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let new_last_modified = resp
        .headers()
        .get(reqwest::header::LAST_MODIFIED)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    let body = resp.bytes().await.context("read constraints body")?;
    let rows = parse_rows(&body, last_ts)?;

    conn.execute(
        "INSERT INTO ingest_http_cache(url, etag, last_modified, fetched_at) \
         VALUES ($1, $2, $3, CURRENT_TIMESTAMP) \
         ON CONFLICT (url) DO UPDATE SET \
           etag = EXCLUDED.etag, last_modified = EXCLUDED.last_modified, \
           fetched_at = EXCLUDED.fetched_at",
        &[&url, &new_etag, &new_last_modified],
    )
    .await
    .context("persist validators")?;

    if rows.is_empty() {
        crate::log_info!("constraints_no_new_rows");
        return Ok(0);
    }

    let cols: Vec<String> = ["constraint_group", "ts", "limit_mw", "flow_mw"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let n = db::upsert(&conn, TABLE, &cols, &rows, &CONFLICT)
        .await
        .context("upsert constraints")?;
    crate::log_info!("constraints_ingested", "rows", n);
    Ok(n)
}

/// Parse the raw CSV body into upsert rows, dropping anything at or below the
/// high-water mark and any row that fails to parse (mirroring Python's
/// KeyError/ValueError/AttributeError catch-and-continue).
///
/// Exposed as `pub` so the integration test suite can exercise the parse core
/// directly; `cmd_refresh_constraints` is the only production caller.
pub fn parse_rows(bytes: &[u8], last_ts: Option<NaiveDateTime>) -> Result<Vec<Row>> {
    let bytes = strip_bom(bytes);
    let mut cache: HashMap<String, NaiveDateTime> = HashMap::new();

    let mut rdr = ReaderBuilder::new().has_headers(true).from_reader(bytes);
    let headers = rdr.headers().context("read constraints CSV header")?.clone();
    let column = |options: &[&str]| -> Result<usize> {
        options
            .iter()
            .find_map(|o| headers.iter().position(|h| h == *o))
            .ok_or_else(|| {
                anyhow!(
                    "constraints CSV has no {} column; header is {:?}",
                    options.join(" / "),
                    headers.iter().collect::<Vec<_>>()
                )
            })
    };
    let i_date = column(HDR_DATE)?;
    let i_group = column(HDR_GROUP)?;
    let i_limit = column(HDR_LIMIT)?;
    let i_flow = column(HDR_FLOW)?;

    let mut rows: Vec<Row> = Vec::new();
    for rec in rdr.records() {
        let rec = match rec {
            Ok(r) => r,
            Err(_) => continue,
        };
        let Some(ts) = parse_ts(rec.get(i_date).unwrap_or("").trim(), &mut cache) else {
            continue;
        };
        if let Some(lt) = last_ts {
            if ts <= lt {
                continue;
            }
        }
        let group = rec.get(i_group).unwrap_or("").trim();
        let Some(limit) = float_cell(rec.get(i_limit).unwrap_or("")) else {
            continue;
        };
        let Some(flow) = float_cell(rec.get(i_flow).unwrap_or("")) else {
            continue;
        };
        rows.push(vec![
            Value::text(group),
            Value::ts(ts),
            limit.map(Value::f).unwrap_or(Value::Null),
            flow.map(Value::f).unwrap_or(Value::Null),
        ]);
    }
    Ok(rows)
}

/// Parse a GMT/BST wall-clock timestamp to naive UTC, memoised on the raw
/// string. Unparseable strings return `None` (row is skipped) and are not cached.
fn parse_ts(raw: &str, cache: &mut HashMap<String, NaiveDateTime>) -> Option<NaiveDateTime> {
    if let Some(ts) = cache.get(raw) {
        return Some(*ts);
    }
    // The forms Python's `datetime.fromisoformat` accepts and the file has
    // used: `T` or space separator, with or without seconds (~11% of the
    // current file is `2023-10-28T00:00`), optional fractional seconds.
    const FORMATS: [&str; 4] = [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M",
    ];
    let naive = FORMATS
        .iter()
        .find_map(|f| NaiveDateTime::parse_from_str(raw, f).ok())?;
    let ts = london_to_utc(naive);
    if let Some(ts) = ts {
        cache.insert(raw.to_string(), ts);
    }
    ts
}

/// Interpret a wall-clock `NaiveDateTime` as Europe/London and return the naive
/// UTC instant. Matches Python's `replace(tzinfo=_LONDON).astimezone(UTC)`:
/// ambiguous (fall-back) times resolve to the earlier occurrence (fold=0) and a
/// non-existent spring-forward time is resolved through the pre-transition
/// (UTC-offset 0) side, so it survives as its own wall clock in UTC.
fn london_to_utc(naive: NaiveDateTime) -> Option<NaiveDateTime> {
    match London.from_local_datetime(&naive) {
        LocalResult::Single(dt) => Some(dt.naive_utc()),
        LocalResult::Ambiguous(earliest, _) => Some(earliest.naive_utc()),
        LocalResult::None => Some(naive),
    }
}

/// Empty/whitespace → `Some(None)` (→ SQL NULL); a parseable float → `Some(Some(f))`;
/// any other non-empty value → `None`, which drops the whole row (Python's ValueError).
fn float_cell(raw: &str) -> Option<Option<f64>> {
    let t = raw.trim();
    if t.is_empty() {
        return Some(None);
    }
    t.parse::<f64>().ok().map(Some)
}

/// Drop the leading UTF-8 BOM so the first CSV header name parses cleanly.
fn strip_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes)
}