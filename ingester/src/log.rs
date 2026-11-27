//! Tiny key=value logger mirroring structlog's KeyValueRenderer output shape
//! (e.g. `2026-08-08T20:04:41.123456Z [info     ] ingested dataset=boalf rows=156481`).

use std::fmt::Display;

fn iso_now() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()
}

pub fn emit(level: &str, event: &str, kv: &[(&str, String)]) {
    let mut parts = vec![format!("{} [{:<9}] {}", iso_now(), level, event)];
    for (k, v) in kv {
        parts.push(format!("{k}={v}"));
    }
    eprintln!("{}", parts.join(" "));
}

#[macro_export]
macro_rules! log_info {
    ($event:literal $(, $k:expr, $v:expr)*) => {
        $crate::log::emit("info", $event, &[$(($k, $v.to_string())),*])
    };
}

#[macro_export]
macro_rules! log_warn {
    ($event:literal $(, $k:expr, $v:expr)*) => {
        $crate::log::emit("warn", $event, &[$(($k, $v.to_string())),*])
    };
}

#[macro_export]
macro_rules! log_error {
    ($event:literal $(, $k:expr, $v:expr)*) => {
        $crate::log::emit("error", $event, &[$(($k, $v.to_string())),*])
    };
}

/// Helper so callers can pass plain `&str` without a manual `.to_string()`.
pub fn kv<'a, T: Display>(k: &'a str, v: T) -> (&'a str, String) {
    (k, v.to_string())
}

pub fn use_tracing() {
    tracing_subscriber::fmt().with_writer(std::io::stderr).init();
}