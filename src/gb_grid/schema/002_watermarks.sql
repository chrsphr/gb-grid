CREATE TABLE IF NOT EXISTS ingest_watermark (
    dataset    TEXT PRIMARY KEY,
    last_ts    TIMESTAMP,
    updated_at TIMESTAMP
);
