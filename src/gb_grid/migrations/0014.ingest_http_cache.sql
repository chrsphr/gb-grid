-- Validators for conditional GETs on bulk file downloads.
--
-- The NESO constraints CSV is the full history (~1M rows) republished daily. We
-- record the ETag / Last-Modified from each successful download so the next
-- refresh can send If-None-Match / If-Modified-Since and skip the transfer and
-- parse entirely when the file hasn't changed.

CREATE TABLE IF NOT EXISTS ingest_http_cache (
    url            TEXT PRIMARY KEY,
    etag           TEXT,
    last_modified  TEXT,
    fetched_at     TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);
