//! gb-grid-ingest: Rust ingester for the GB grid BMRS data pipeline.
//!
//! Owns the whole ingest loop — fetch → parse → write → watermark — plus the
//! per-BMU dispatch materialization, as a drop-in replacement for the Python
//! `gb-grid` scheduler/ingest side. Python keeps only `migrate`.

#![allow(dead_code)]

pub mod bmrs;
pub mod config;
pub mod constraints;
pub mod db;
pub mod http;
pub mod ingest;
pub mod interp;
pub mod log;
pub mod materialize;
pub mod scheduler;