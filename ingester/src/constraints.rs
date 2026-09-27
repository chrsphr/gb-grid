//! NESO day-ahead constraint flows: streaming CSV refresh with conditional GET.
//!
//! IMPLEMENT THIS MODULE. Do **not** change the public items below — they are
//! consumed by `main.rs` and the nix systemd timer. You may add private helpers.
//!
//! Port of `src/gb_grid/ingest/constraints.py`.
//!
//! Key semantics to preserve:
//! - `CSV_URL` is the fixed NESO resource URL (follows redirects).
//! - Send `If-None-Match` / `If-Modified-Since` validators read from the
//!   `ingest_http_cache` table; a 304 means unchanged → return 0 rows.
//! - Stream the body line by line (do not buffer the whole file) — the CSV is
//!   the full history (~1M rows, several hundred MB).
//! - Drop the UTF-8 BOM from the first line so the header parses.
//! - Parse `Date (GMT/BST)` as Europe/London wall-clock → naive UTC, memoised on
//!   the raw string (each timestamp repeats across every constraint group).
//! - Filter `ts <= last_ts` (the high-water mark = `MAX(ts)` from the table)
//!   *before* building rows — on a daily refresh nearly all rows are dropped.
//! - `Limit (MW)` / `Flow (MW)` → float-or-null; skip rows that fail to parse.
//! - On success, persist the new validators back into `ingest_http_cache`.
//! - Upsert via `db::upsert` with table `constraints`, conflict `constraint_group, ts`.
//!
//! The download itself has **no** retry (matching the Python httpx path); use
//! `crate::http::Http::stream` with a 120s timeout.

use anyhow::Result;

use crate::config::Config;

pub const CSV_URL: &str = "https://api.neso.energy/dataset/cf3cbc92-2d5d-4c2b-bd29-e11a21070b26/resource/38a18ec1-9e40-465d-93fb-301e80fd1352/download/day-ahead-constraints-limits-and-flow-output-v1.5.csv";

pub const TABLE: &str = "constraints";
pub const CONFLICT: [&str; 2] = ["constraint_group", "ts"];

/// CLI `refresh-constraints` handler. Downloads the CSV, upserts new rows, and
/// returns the number of rows written (0 for an unchanged file).
pub async fn cmd_refresh_constraints(cfg: &Config) -> Result<usize> {
    todo!("implement cmd_refresh_constraints")
}