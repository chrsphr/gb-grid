//! HTTP transport: a retrying client for the BMRS API and a raw streaming path
//! for the constraints CSV download.
//!
//! Retry semantics mirror `src/gb_grid/api/client.py`: only transport errors and
//! 429/5xx are retried, with exponential backoff (multiplier 1, min 1s, max 30s)
//! over at most 5 attempts. Other 4xx fail fast — a bad window will never
//! succeed no matter how many times it's retried.

use std::time::Duration;

use anyhow::{anyhow, Result};
use serde_json::Value;

use crate::config::Config;

#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("http transport error: {0}")]
    Transport(String),
    #[error("http status {status}: {body}")]
    Status { status: u16, body: String },
}

impl HttpError {
    fn retryable(&self) -> bool {
        match self {
            HttpError::Transport(_) => true,
            HttpError::Status { status, .. } => *status == 429 || *status >= 500,
        }
    }
}

pub struct Http {
    client: reqwest::Client,
    base: String,
    timeout: Duration,
}

impl Http {
    pub fn new(cfg: &Config) -> Http {
        Http {
            client: build_client(),
            base: cfg.api_base.clone(),
            timeout: cfg.http_timeout,
        }
    }

    /// GET a JSON endpoint with retry. `params` are appended as a query string.
    pub async fn get(&self, path: &str, params: &[(&str, String)]) -> Result<Value> {
        let url = format!("{}{}", self.base, path);
        let mut attempt = 0u32;
        loop {
            match self.try_get(&url, params).await {
                Ok(json) => return Ok(json),
                Err(e) if e.retryable() && attempt < 4 => {
                    attempt += 1;
                    let backoff = (1u64 << attempt).min(30); // 2,4,8,16 for attempts 1..=4
                    tokio::time::sleep(Duration::from_secs(backoff)).await;
                }
                Err(e) => return Err(anyhow!("{e}")),
            }
        }
    }

    /// Raw streaming GET (no retry) — used by the constraints CSV refresh.
    /// Returns the full `reqwest::Response` so the caller can inspect status
    /// (304) and stream the body line by line.
    pub async fn stream(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        timeout: Duration,
    ) -> Result<reqwest::Response> {
        let mut req = self.client.get(url).timeout(timeout);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        req.send().await.map_err(|e| anyhow!("{e}"))
    }

    async fn try_get(&self, url: &str, params: &[(&str, String)]) -> Result<Value, HttpError> {
        let resp = self
            .client
            .get(url)
            .query(params)
            .timeout(self.timeout)
            .send()
            .await
            .map_err(|e| HttpError::Transport(e.to_string()))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp
                .text()
                .await
                .unwrap_or_default()
                .chars()
                .take(500)
                .collect::<String>();
            return Err(HttpError::Status {
                status: status.as_u16(),
                body,
            });
        }
        resp.json().await.map_err(|e| HttpError::Transport(e.to_string()))
    }
}

pub fn build_client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .user_agent("gb-grid-ingest/0.1")
        .build()
        .expect("failed to build http client")
}