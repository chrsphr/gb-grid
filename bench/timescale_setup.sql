-- TimescaleDB adoption: hypertables + continuous aggregates.
-- Applied to the gb_grid_ts clone for benchmarking; folded into a yoyo
-- migration once the numbers justify it.

CREATE EXTENSION IF NOT EXISTS timescaledb;

-- Every table's PK already contains its time column, so no PK rework needed.
-- Smaller chunks (1 day) on the tables dashboards range-scan most heavily give
-- tighter chunk exclusion; 7-day chunks elsewhere keep chunk counts sane.
SELECT create_hypertable('bmu_dispatch', 'ts',
       chunk_time_interval => INTERVAL '1 day', migrate_data => true, if_not_exists => true);
-- constraints spans years (NESO day-ahead, ~400 rows/day), so a wide chunk
-- keeps the chunk count sane — 1-day here shattered it into 2000+ chunks.
SELECT create_hypertable('constraints', 'ts',
       chunk_time_interval => INTERVAL '30 days', migrate_data => true, if_not_exists => true);
SELECT create_hypertable('pn', 'time_from',
       chunk_time_interval => INTERVAL '7 days', migrate_data => true, if_not_exists => true);
SELECT create_hypertable('mels', 'time_from',
       chunk_time_interval => INTERVAL '7 days', migrate_data => true, if_not_exists => true);
SELECT create_hypertable('boalf', 'time_from',
       chunk_time_interval => INTERVAL '7 days', migrate_data => true, if_not_exists => true);
SELECT create_hypertable('b1610', 'settlement_date',
       chunk_time_interval => INTERVAL '7 days', migrate_data => true, if_not_exists => true);
SELECT create_hypertable('fuelinst', 'publish_time',
       chunk_time_interval => INTERVAL '7 days', migrate_data => true, if_not_exists => true);

-- Continuous aggregates replace the hand-rolled `materialize_dispatch_daily`.
-- Store raw SUMs; the MW->MWh scaling lives in the view so the materialized
-- data is independent of the 5-min sample interval constant.
CREATE MATERIALIZED VIEW bmu_dispatch_daily_cagg
WITH (timescaledb.continuous) AS
SELECT bmu,
       time_bucket(INTERVAL '1 day', ts) AS bucket,
       SUM(pn_mw)              AS pn_mw_sum,
       SUM(boa_level_mw)       AS boa_mw_sum,
       SUM(so_turnup_mw)       AS so_turnup_sum,
       SUM(boa_curtailment_mw) AS boa_curt_sum,
       SUM(so_curtailment_mw)  AS so_curt_sum
FROM bmu_dispatch
GROUP BY bmu, time_bucket(INTERVAL '1 day', ts)
WITH NO DATA;

CREATE MATERIALIZED VIEW b1610_daily_cagg
WITH (timescaledb.continuous) AS
SELECT ngc_bm_unit AS bmu,
       time_bucket(INTERVAL '1 day', settlement_date) AS bucket,
       SUM(quantity_mwh) AS b1610_mwh
FROM b1610
WHERE ngc_bm_unit IS NOT NULL
GROUP BY ngc_bm_unit, time_bucket(INTERVAL '1 day', settlement_date)
WITH NO DATA;

-- Initial full materialization.
CALL refresh_continuous_aggregate('bmu_dispatch_daily_cagg', NULL, NULL);
CALL refresh_continuous_aggregate('b1610_daily_cagg', NULL, NULL);

-- Replace the table with a view of identical shape so dashboards are unchanged.
DROP TABLE IF EXISTS bmu_dispatch_daily;
CREATE VIEW bmu_dispatch_daily AS
SELECT COALESCE(d.bmu, a.bmu)               AS bmu,
       COALESCE(d.bucket::date, a.bucket)   AS date,
       d.pn_mw_sum     * (5.0/60.0)         AS pn_mwh,
       d.boa_mw_sum    * (5.0/60.0)         AS boa_dispatched_mwh,
       d.so_turnup_sum * (5.0/60.0)         AS so_turnup_mwh,
       d.boa_curt_sum  * (5.0/60.0)         AS boa_curtailment_mwh,
       d.so_curt_sum   * (5.0/60.0)         AS so_curtailment_mwh,
       a.b1610_mwh                          AS b1610_mwh
FROM bmu_dispatch_daily_cagg d
FULL OUTER JOIN b1610_daily_cagg a
  ON a.bmu = d.bmu AND a.bucket = d.bucket::date;
