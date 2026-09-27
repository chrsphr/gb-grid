//! Shared integration-test helpers: bootstrap a throwaway scratch database.
//!
//! Mirrors the Python test setup (`tests/conftest.py`): the dev compose
//! Postgres is the admin, a uniquely-named sibling database is created per test
//! process, the SQL migrations are applied via the still-installed Python
//! `gb-grid migrate` (migration 0004 is a Python yoyo step the Rust harness
//! can't run itself), and each test gets a connection to it.
//!
//! All helpers are **skip-safe**: if `GB_GRID_DATABASE_URL` is unset (e.g. a
//! plain `cargo test` outside `nix develop`) the DB-backed tests return `None`
//! and the test body returns early instead of failing.

use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio_postgres::{Client, NoTls};

use gb_grid_ingest::config::{Config, PollConfig};

/// The per-process scratch database name. Unique per PID so concurrently-running
/// `cargo test` processes (one per agent) don't collide.
fn db_name() -> String {
    format!("gb_grid_test_rs_{}", std::process::id())
}

fn swap_db(url: &str, db: &str) -> String {
    let (base, query) = match url.split_once('?') {
        Some((b, q)) => (b, format!("?{q}")),
        None => (url, String::new()),
    };
    let idx = base.rfind('/').unwrap_or(base.len() - 1);
    format!("{}{}{}", &base[..=idx], db, query)
}

fn repo_root() -> &'static str {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("ingester/ should be a direct child of the repo root")
        .to_str()
        .expect("repo root is valid utf-8")
}

fn migrate(url: &str) -> Result<()> {
    let root = repo_root();
    let status = std::process::Command::new(format!("{root}/.venv/bin/gb-grid"))
        .arg("migrate")
        .env("GB_GRID_DATABASE_URL", url)
        .status()
        .with_context(|| "failed to spawn gb-grid migrate")?;
    if !status.success() {
        anyhow::bail!("gb-grid migrate exited {status}");
    }
    Ok(())
}

fn bootstrap() -> Option<String> {
    let base = std::env::var("GB_GRID_DATABASE_URL").ok()?;
    let admin = swap_db(&base, "postgres");
    let name = db_name();
    let url = swap_db(&base, &name);

    // Run the admin DB work on a dedicated thread: tests may call this from
    // inside a tokio runtime, where block_on on a *new* runtime would panic
    // ("Cannot start a runtime from within a runtime").
    let created = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .ok()?;
        rt.block_on(async move {
            let (conn, connection) = tokio_postgres::connect(&admin, NoTls).await.ok()?;
            tokio::spawn(async move {
                let _ = connection.await;
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
    if migrate(&url).is_err() {
        return None;
    }
    Some(url)
}

/// The scratch database URL for this process, or `None` to skip DB tests.
pub fn url() -> Option<String> {
    static URL: OnceLock<Option<String>> = OnceLock::new();
    URL.get_or_init(|| bootstrap()).clone()
}

/// A `Config` pointed at the scratch database (HTTP base is unused for DB tests).
pub fn config() -> Option<Config> {
    let url = url()?;
    Some(Config {
        database_url: url,
        api_base: "http://127.0.0.1:1".to_string(),
        http_timeout: Duration::from_secs(60),
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
    })
}

/// A connection to the scratch database, or `None` to skip DB tests.
pub async fn connect() -> Option<Client> {
    let url = url()?;
    gb_grid_ingest::db::connect(&config()?).await.ok()
}

/// Truncate the given tables (sibling of the Python conftest truncation).
pub async fn truncate(conn: &Client, tables: &[&str]) {
    for t in tables {
        conn.execute(&format!("TRUNCATE {t}"), &[])
            .await
            .unwrap_or_else(|e| panic!("truncate {t}: {e}"));
    }
}