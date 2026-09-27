//! Unit + DB-backed tests for the ingest orchestration (`run_windows` and the
//! window generators). DB tests mirror `tests/test_run_windows.py` and skip
//! gracefully when `GB_GRID_DATABASE_URL` is unset.

mod common;

use std::path::Path;
use std::sync::OnceLock;
use tokio_postgres::{Client, NoTls};

use chrono::{Duration, NaiveDate, NaiveDateTime, Timelike};
use gb_grid_ingest::config::{Config, PollConfig};
use gb_grid_ingest::db::{self, Row, Value};
use gb_grid_ingest::ingest::{daterange, datetime_range, run_windows, IngestResult};
use std::sync::Arc;

fn dt(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(y, mo, d)
        .unwrap()
        .and_hms_opt(h, mi, s)
        .unwrap()
}

fn base() -> NaiveDateTime {
    dt(2026, 5, 1, 0, 0, 0)
}

fn windows() -> Vec<(NaiveDateTime, NaiveDateTime)> {
    let b = base();
    (0..8).map(|i| (b + Duration::hours(i), b + Duration::hours(i + 1))).collect()
}

fn fuel_cols() -> Vec<String> {
    [
        "publish_time",
        "settlement_date",
        "settlement_period",
        "fuel_type",
        "generation_mw",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn fuel_row(publish: NaiveDateTime, gen: f64) -> Row {
    vec![
        Value::Timestamp(publish),
        Value::Null,
        // settlement_period is SMALLINT; db::upsert's VALUES path can't bind
        // Value::Int(i64) to int2, and the column is nullable, so stay null.
        Value::Null,
        Value::Text("WIND".to_string()),
        Value::Float(gen),
    ]
}

/// Standard single-row-per-window fetch, keyed on the window start.
fn simple_fetch(
    cols: Vec<String>,
) -> impl Fn(
    (NaiveDateTime, NaiveDateTime),
) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = anyhow::Result<(Vec<String>, Vec<Row>)>> + Send>,
> {
    move |w: (NaiveDateTime, NaiveDateTime)| {
        let (s, _e) = w;
        let cols = cols.clone();
        Box::pin(async move { Ok((cols, vec![fuel_row(s, s.hour() as f64)])) })
    }
}

// ---------------------------------------------------------------------------
// Scratch-DB setup
// ---------------------------------------------------------------------------

fn poll_cfg() -> PollConfig {
    PollConfig {
        fuelinst_seconds: 3600,
        boalf_seconds: 3600,
        b1610_seconds: 3600,
        pn_seconds: 3600,
        mels_seconds: 3600,
        system_prices_seconds: 3600,
        materialize_seconds: 3600,
        materialize_daily_seconds: 3600,
    }
}

fn base_config(database_url: String) -> Config {
    Config {
        database_url,
        api_base: "http://127.0.0.1:1".to_string(),
        http_timeout: std::time::Duration::from_secs(60),
        fetch_concurrency: 4,
        poll: poll_cfg(),
    }
}

fn swap_db(url: &str, db: &str) -> String {
    let (base, query) = match url.split_once('?') {
        Some((b, q)) => (b, format!("?{q}")),
        None => (url, String::new()),
    };
    let idx = base.rfind('/').unwrap_or(base.len() - 1);
    format!("{}{}{}", &base[..=idx], db, query)
}

/// Fallback scratch-DB bootstrap used only when `common` skips despite
/// `GB_GRID_DATABASE_URL` being set. The committed `tests/common/mod.rs`
/// `bootstrap()` binds the tokio-postgres `Connection` future to `_`, so the
/// admin connection closes before the scratch database can be created — the
/// `OnceLock` then caches `None` for the whole process. This drives the
/// `Connection` future properly so the DB-backed tests actually run.
fn fallback_url() -> Option<String> {
    static URL: OnceLock<Option<String>> = OnceLock::new();
    URL.get_or_init(|| {
        let base = std::env::var("GB_GRID_DATABASE_URL").ok()?;
        let name = format!("gb_grid_test_rs_{}", std::process::id());
        let admin = swap_db(&base, "postgres");
        let url = swap_db(&base, &name);
        let created = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .ok()?;
            rt.block_on(async move {
                let (conn, conn_task) = tokio_postgres::connect(&admin, NoTls).await.ok()?;
                tokio::spawn(async move {
                    let _ = conn_task.await;
                });
                conn.execute(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"), &[])
                    .await
                    .ok()?;
                conn.execute(&format!("CREATE DATABASE {name}"), &[])
                    .await
                    .ok()?;
                Some(())
            })
        })
        .join()
        .ok()??;
        let _ = created;
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_str()
            .unwrap();
        let ok = std::process::Command::new(format!("{root}/.venv/bin/gb-grid"))
            .arg("migrate")
            .env("GB_GRID_DATABASE_URL", &url)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            return None;
        }
        Some(url)
    })
    .clone()
}

/// `(Config, Client)` for the throwaway scratch database, or `None` to skip.
async fn setup() -> Option<(Config, Client)> {
    if let Some(cfg) = common::config() {
        if let Some(conn) = common::connect().await {
            return Some((cfg, conn));
        }
    }
    let url = fallback_url()?;
    let cfg = base_config(url);
    let conn = db::connect(&cfg).await.ok()?;
    Some((cfg, conn))
}

async fn truncate_fuelinst(conn: &Client) {
    common::truncate(conn, &["fuelinst"]).await;
}

// ---------------------------------------------------------------------------
// daterange / datetime_range
// ---------------------------------------------------------------------------

#[test]
fn daterange_steps_single_day() {
    let start = NaiveDate::from_ymd_opt(2026, 5, 1).unwrap();
    let end = NaiveDate::from_ymd_opt(2026, 5, 3).unwrap();
    assert_eq!(
        daterange(start, end, 1),
        vec![
            (start, start),
            (start + Duration::days(1), start + Duration::days(1)),
            (start + Duration::days(2), start + Duration::days(2)),
        ]
    );
}

#[test]
fn daterange_steps_multiple_days_with_partial_tail() {
    let start = NaiveDate::from_ymd_opt(2026, 5, 1).unwrap();
    let end = NaiveDate::from_ymd_opt(2026, 5, 5).unwrap();
    assert_eq!(
        daterange(start, end, 2),
        vec![
            (start, start + Duration::days(1)),
            (start + Duration::days(2), start + Duration::days(3)),
            (start + Duration::days(4), start + Duration::days(4)),
        ]
    );
}

#[test]
fn daterange_empty_when_start_after_end() {
    let start = NaiveDate::from_ymd_opt(2026, 5, 3).unwrap();
    let end = NaiveDate::from_ymd_opt(2026, 5, 1).unwrap();
    assert!(daterange(start, end, 1).is_empty());
}

#[test]
fn datetime_range_half_open_windows() {
    let start = dt(2026, 5, 1, 0, 0, 0);
    let end = dt(2026, 5, 1, 6, 0, 0);
    let step = Duration::hours(2);
    assert_eq!(
        datetime_range(start, end, step),
        vec![
            (dt(2026, 5, 1, 0, 0, 0), dt(2026, 5, 1, 2, 0, 0)),
            (dt(2026, 5, 1, 2, 0, 0), dt(2026, 5, 1, 4, 0, 0)),
            (dt(2026, 5, 1, 4, 0, 0), dt(2026, 5, 1, 6, 0, 0)),
        ]
    );
}

#[test]
fn datetime_range_uneven_tail() {
    let start = dt(2026, 5, 1, 0, 0, 0);
    let end = dt(2026, 5, 1, 5, 0, 0);
    let step = Duration::hours(2);
    assert_eq!(
        datetime_range(start, end, step),
        vec![
            (dt(2026, 5, 1, 0, 0, 0), dt(2026, 5, 1, 2, 0, 0)),
            (dt(2026, 5, 1, 2, 0, 0), dt(2026, 5, 1, 4, 0, 0)),
            (dt(2026, 5, 1, 4, 0, 0), dt(2026, 5, 1, 5, 0, 0)),
        ]
    );
}

#[test]
fn datetime_range_empty_when_start_equals_end() {
    let t = dt(2026, 5, 1, 0, 0, 0);
    assert!(datetime_range(t, t, Duration::days(1)).is_empty());
}

// ---------------------------------------------------------------------------
// run_windows (DB-backed)
// ---------------------------------------------------------------------------

// `cargo test` runs `#[tokio::test]` bodies on parallel threads, all sharing
// the single per-process scratch database. Serialize the DB tests so they
// don't truncate/interleave each other's rows (the Python suite runs its tests
// sequentially).
static DB_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lock_db() -> std::sync::MutexGuard<'static, ()> {
    DB_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

#[tokio::test]
async fn concurrent_matches_serial() {
    let _db_guard = lock_db();
    let Some((cfg, conn)) = setup().await else {
        eprintln!("SKIP: GB_GRID_DATABASE_URL not set");
        return;
    };

    let cols = fuel_cols();
    let fetch = simple_fetch(cols);

    truncate_fuelinst(&conn).await;
    let serial = run_windows(
        &conn,
        &cfg,
        "fuelinst",
        "fuelinst",
        &["publish_time", "fuel_type"],
        windows(),
        &fetch,
        Some("publish_time"),
        Some(1),
    )
    .await
    .unwrap();
    let serial_table = table_contents(&conn).await;

    truncate_fuelinst(&conn).await;
    let parallel = run_windows(
        &conn,
        &cfg,
        "fuelinst",
        "fuelinst",
        &["publish_time", "fuel_type"],
        windows(),
        &fetch,
        Some("publish_time"),
        Some(4),
    )
    .await
    .unwrap();
    let parallel_table = table_contents(&conn).await;

    assert_eq!(serial.rows, windows().len());
    assert_eq!(parallel.rows, windows().len());
    assert_eq!(serial.last_window_with_data, Some(*windows().last().unwrap()));
    assert_eq!(parallel.last_window_with_data, Some(*windows().last().unwrap()));
    assert_eq!(
        serial_table, parallel_table,
        "concurrency 1 and 4 must produce identical tables"
    );

    let wm = db::get_watermark(&conn, "fuelinst").await.unwrap();
    assert_eq!(wm, Some(windows().last().unwrap().0));
}

#[tokio::test]
async fn fetches_actually_run_concurrently() {
    let _db_guard = lock_db();
    let Some((cfg, conn)) = setup().await else {
        eprintln!("SKIP: GB_GRID_DATABASE_URL not set");
        return;
    };

    // A barrier of 4 that never releases unless 4 fetches are in flight at once.
    let barrier = Arc::new(tokio::sync::Barrier::new(4));
    let cols = fuel_cols();
    let fetch = {
        let barrier = Arc::clone(&barrier);
        move |w: (NaiveDateTime, NaiveDateTime)| {
            let barrier = Arc::clone(&barrier);
            let cols = cols.clone();
            let (s, _e) = w;
            async move {
                barrier.wait().await;
                Ok::<_, anyhow::Error>((cols, vec![fuel_row(s, s.hour() as f64)]))
            }
        }
    };

    truncate_fuelinst(&conn).await;
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        run_windows(
            &conn,
            &cfg,
            "fuelinst",
            "fuelinst",
            &["publish_time", "fuel_type"],
            windows(),
            fetch,
            Some("publish_time"),
            Some(4),
        ),
    )
    .await
    .expect("fetches did not run concurrently (barrier never released)")
    .unwrap();

    assert_eq!(result.rows, windows().len());
}

#[tokio::test]
async fn last_window_with_data_skips_empty_tail() {
    let _db_guard = lock_db();
    let Some((cfg, conn)) = setup().await else {
        eprintln!("SKIP: GB_GRID_DATABASE_URL not set");
        return;
    };

    let cols = fuel_cols();
    let fetch = move |w: (NaiveDateTime, NaiveDateTime)| {
        let (s, _e) = w;
        let cols = cols.clone();
        async move {
            if s.hour() >= 5 {
                Ok::<_, anyhow::Error>((cols, vec![]))
            } else {
                Ok::<_, anyhow::Error>((cols, vec![fuel_row(s, s.hour() as f64)]))
            }
        }
    };

    truncate_fuelinst(&conn).await;
    let result: IngestResult<(NaiveDateTime, NaiveDateTime)> = run_windows(
        &conn,
        &cfg,
        "fuelinst",
        "fuelinst",
        &["publish_time", "fuel_type"],
        windows(),
        fetch,
        Some("publish_time"),
        Some(4),
    )
    .await
    .unwrap();

    assert_eq!(result.rows, 5);
    assert_eq!(result.last_window_with_data, Some(windows()[4]));

    let wm = db::get_watermark(&conn, "fuelinst").await.unwrap();
    assert_eq!(wm, Some(windows()[4].0));
}

#[tokio::test]
async fn writes_stay_ordered_under_concurrency() {
    let _db_guard = lock_db();
    let Some((cfg, conn)) = setup().await else {
        eprintln!("SKIP: GB_GRID_DATABASE_URL not set");
        return;
    };

    // Every window emits a row with the SAME conflict key (publish_time=BASE,
    // fuel_type=WIND); whichever window is written last must win the conflict.
    let cols = fuel_cols();
    let b = base();
    let fetch = move |w: (NaiveDateTime, NaiveDateTime)| {
        let (s, _e) = w;
        let cols = cols.clone();
        async move {
            let mut row = fuel_row(b, s.hour() as f64);
            row[0] = Value::Timestamp(b);
            Ok::<_, anyhow::Error>((cols, vec![row]))
        }
    };

    truncate_fuelinst(&conn).await;
    run_windows(
        &conn,
        &cfg,
        "fuelinst",
        "fuelinst",
        &["publish_time", "fuel_type"],
        windows(),
        fetch,
        Some("publish_time"),
        Some(4),
    )
    .await
    .unwrap();

    let rows = conn
        .query("SELECT generation_mw FROM fuelinst", &[])
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get::<_, f64>(0), windows().last().unwrap().0.hour() as f64);
}

async fn table_contents(conn: &Client) -> Vec<(i64, f64)> {
    conn.query(
        "SELECT publish_time, generation_mw FROM fuelinst ORDER BY publish_time",
        &[],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| {
        let ts: chrono::NaiveDateTime = r.get(0);
        let gen: f64 = r.get(1);
        (ts.and_utc().timestamp(), gen)
    })
    .collect()
}