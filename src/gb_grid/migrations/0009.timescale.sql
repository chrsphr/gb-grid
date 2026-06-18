-- TimescaleDB adoption: hypertables, continuous aggregates, compression.
--
-- transactional: false
--
-- Runs without a wrapping transaction because continuous-aggregate creation,
-- refresh, and policy calls cannot execute inside a transaction block.
--
-- NOTE (operational): create_hypertable(..., migrate_data => true) rewrites each
-- table into chunks and takes an exclusive lock. On the live DB (~1.5 GB for
-- bmu_dispatch) this is minutes of downtime — stop the ingester before applying.
--
-- Every PK already contains its time column, so no PK rework is needed.
-- bmu_dispatch_daily stays a plain (indexed) table; it is now refreshed cheaply
-- from the two continuous aggregates by materialize_dispatch_daily(), instead of
-- scanning the raw 9.7M-row bmu_dispatch. Day buckets switch from the previous
-- session-TZ-localised boundary to true UTC (bug fix; heatmap totals shift slightly).

CREATE EXTENSION IF NOT EXISTS timescaledb;

-- Chunk intervals tuned to data density: dense 5-min dispatch -> 1 day; the
-- multi-year constraints table -> 30 days (1-day shattered it into 2000+ chunks);
-- the rest -> 7 days.
SELECT create_hypertable('bmu_dispatch', 'ts',
       chunk_time_interval => INTERVAL '1 day', migrate_data => true, if_not_exists => true);
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

-- Continuous aggregates: daily per-BMU SUMs (raw MW; MWh scaling done by the
-- consumer so materialized data is independent of the 5-min sample constant).
CREATE MATERIALIZED VIEW IF NOT EXISTS bmu_dispatch_daily_cagg
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

CREATE MATERIALIZED VIEW IF NOT EXISTS b1610_daily_cagg
WITH (timescaledb.continuous) AS
SELECT ngc_bm_unit AS bmu,
       time_bucket(INTERVAL '1 day', settlement_date) AS bucket,
       SUM(quantity_mwh) AS b1610_mwh
FROM b1610
WHERE ngc_bm_unit IS NOT NULL
GROUP BY ngc_bm_unit, time_bucket(INTERVAL '1 day', settlement_date)
WITH NO DATA;

CALL refresh_continuous_aggregate('bmu_dispatch_daily_cagg', NULL, NULL);
CALL refresh_continuous_aggregate('b1610_daily_cagg', NULL, NULL);

-- Auto-refresh going forward. Wide start_offset so late B1610 actuals (~5
-- working-day lag) and dispatch revisions keep getting folded in.
SELECT add_continuous_aggregate_policy('bmu_dispatch_daily_cagg',
       start_offset => INTERVAL '30 days', end_offset => INTERVAL '1 hour',
       schedule_interval => INTERVAL '30 minutes', if_not_exists => true);
SELECT add_continuous_aggregate_policy('b1610_daily_cagg',
       start_offset => INTERVAL '30 days', end_offset => INTERVAL '1 day',
       schedule_interval => INTERVAL '30 minutes', if_not_exists => true);

-- Columnar compression. Policy threshold sits beyond the revision/dashboard
-- window (30 days) so hot chunks stay row-store and fast to query/upsert;
-- only cold chunks compress (~24x smaller in benchmarking).
ALTER TABLE bmu_dispatch SET (timescaledb.compress,
      timescaledb.compress_segmentby = 'bmu', timescaledb.compress_orderby = 'ts');
ALTER TABLE pn SET (timescaledb.compress,
      timescaledb.compress_segmentby = 'national_grid_bm_unit', timescaledb.compress_orderby = 'time_from');
ALTER TABLE mels SET (timescaledb.compress,
      timescaledb.compress_segmentby = 'national_grid_bm_unit', timescaledb.compress_orderby = 'time_from');
ALTER TABLE boalf SET (timescaledb.compress,
      timescaledb.compress_segmentby = 'bm_unit', timescaledb.compress_orderby = 'time_from');
ALTER TABLE b1610 SET (timescaledb.compress,
      timescaledb.compress_segmentby = 'ngc_bm_unit', timescaledb.compress_orderby = 'settlement_period');
ALTER TABLE constraints SET (timescaledb.compress,
      timescaledb.compress_segmentby = 'constraint_group', timescaledb.compress_orderby = 'ts');

SELECT add_compression_policy('bmu_dispatch', INTERVAL '30 days', if_not_exists => true);
SELECT add_compression_policy('pn',           INTERVAL '30 days', if_not_exists => true);
SELECT add_compression_policy('mels',         INTERVAL '30 days', if_not_exists => true);
SELECT add_compression_policy('boalf',        INTERVAL '30 days', if_not_exists => true);
SELECT add_compression_policy('b1610',        INTERVAL '30 days', if_not_exists => true);
SELECT add_compression_policy('constraints',  INTERVAL '60 days', if_not_exists => true);
