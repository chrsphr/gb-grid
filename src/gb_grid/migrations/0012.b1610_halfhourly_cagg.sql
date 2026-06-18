-- Half-hourly per-BMU B1610 continuous aggregate, for the per-station output
-- heatmap dashboard.
--
-- transactional: false
--
-- Runs without a wrapping transaction: continuous-aggregate creation, refresh,
-- and policy calls cannot execute inside a transaction block.
--
-- Why: the heatmap needs per-BMU actual output at half-hourly grain across many
-- stations at once. Querying that from raw b1610 meant decompressing ~3M
-- columnar rows for the whole week on every panel (~60s each) — unusable live.
-- This materialized view serves the same grain without touching the compressed
-- source.
--
-- b1610's hypertable time dimension is settlement_date (a DATE), so time_bucket
-- can only bucket at >= 1 day. The half-hour lives in settlement_period (1-48),
-- which we keep as an extra GROUP BY column to retain full half-hourly grain.
-- Consumers rebuild the timestamp as bucket + (settlement_period - 1) * 30 min.
-- MWh -> MW scaling (*2) is left to the consumer, matching b1610_daily_cagg.

CREATE MATERIALIZED VIEW IF NOT EXISTS b1610_hh_cagg
WITH (timescaledb.continuous) AS
SELECT ngc_bm_unit AS bmu,
       time_bucket(INTERVAL '1 day', settlement_date) AS bucket,
       settlement_period,
       SUM(quantity_mwh) AS b1610_mwh
FROM b1610
WHERE ngc_bm_unit IS NOT NULL
GROUP BY ngc_bm_unit, time_bucket(INTERVAL '1 day', settlement_date), settlement_period
WITH NO DATA;

CALL refresh_continuous_aggregate('b1610_hh_cagg', NULL, NULL);

-- Wide start_offset so late B1610 actuals (~5 working-day lag) keep folding in,
-- mirroring the b1610_daily_cagg policy.
SELECT add_continuous_aggregate_policy('b1610_hh_cagg',
       start_offset => INTERVAL '30 days', end_offset => INTERVAL '1 day',
       schedule_interval => INTERVAL '30 minutes', if_not_exists => true);
