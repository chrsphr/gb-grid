//! Integration tests for the scheduler module's CLI handlers.
//!
//! DB-backed tests are skip-guarded via `common`: they return early (printing
//! SKIP) when `GB_GRID_DATABASE_URL` is unset. The scratch database carries the
//! continuous aggregates from migrations 0009-0012, so a tiny refresh window is
//! enough to exercise `cmd_refresh_caggs` end-to-end.

mod common;

use gb_grid_ingest::scheduler;

#[tokio::test]
async fn cmd_status_reports_against_scratch_db() {
    let Some(cfg) = common::config() else {
        eprintln!("SKIP: GB_GRID_DATABASE_URL not set");
        return;
    };
    // Prints the table stats + watermarks lines; must not error on a fresh DB.
    scheduler::cmd_status(&cfg).await.expect("cmd_status should succeed");
}

#[tokio::test]
async fn cmd_refresh_caggs_over_tiny_window() {
    let Some(cfg) = common::config() else {
        eprintln!("SKIP: GB_GRID_DATABASE_URL not set");
        return;
    };
    scheduler::cmd_refresh_caggs(&cfg, "2026-09-01", "2026-09-02")
        .await
        .expect("cmd_refresh_caggs should succeed");
}