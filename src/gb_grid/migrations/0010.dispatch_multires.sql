-- Multi-resolution dispatch serving.
--
-- transactional: false
--
-- (continuous-aggregate creation, refresh and policy calls can't run inside a
-- transaction block.)
--
-- Two continuous aggregates on the bmu_dispatch hypertable (hourly + daily avg MW)
-- plus dispatch_series(), a router that serves the dispatch dashboard at a fidelity
-- matched to the zoom level: raw 5-min when zoomed in, hourly / daily when zoomed
-- out. Grafana passes the bucket width via $__interval, so every zoom returns
-- ~1-2k points and stays in the low-ms range. The daily MWh heatmap is unaffected
-- (it keeps using bmu_dispatch_daily).

CREATE MATERIALIZED VIEW IF NOT EXISTS bmu_dispatch_hourly
WITH (timescaledb.continuous) AS
SELECT bmu, time_bucket(interval '1 hour', ts) AS bucket,
  avg(pn_mw) AS pn_mw, avg(boa_level_mw) AS boa_level_mw, avg(mel_mw) AS mel_mw,
  avg(so_turnup_mw) AS so_turnup_mw, avg(boa_curtailment_mw) AS boa_curtailment_mw,
  avg(so_curtailment_mw) AS so_curtailment_mw
FROM bmu_dispatch GROUP BY bmu, time_bucket(interval '1 hour', ts)
WITH NO DATA;

CREATE MATERIALIZED VIEW IF NOT EXISTS bmu_dispatch_daily_mw
WITH (timescaledb.continuous) AS
SELECT bmu, time_bucket(interval '1 day', ts) AS bucket,
  avg(pn_mw) AS pn_mw, avg(boa_level_mw) AS boa_level_mw, avg(mel_mw) AS mel_mw,
  avg(so_turnup_mw) AS so_turnup_mw, avg(boa_curtailment_mw) AS boa_curtailment_mw,
  avg(so_curtailment_mw) AS so_curtailment_mw
FROM bmu_dispatch GROUP BY bmu, time_bucket(interval '1 day', ts)
WITH NO DATA;

CALL refresh_continuous_aggregate('bmu_dispatch_hourly', NULL, NULL);
CALL refresh_continuous_aggregate('bmu_dispatch_daily_mw', NULL, NULL);

-- Wide start_offset so dispatch revisions keep folding into the rollups.
SELECT add_continuous_aggregate_policy('bmu_dispatch_hourly',
  start_offset => INTERVAL '30 days', end_offset => INTERVAL '1 hour',
  schedule_interval => INTERVAL '30 minutes', if_not_exists => true);
SELECT add_continuous_aggregate_policy('bmu_dispatch_daily_mw',
  start_offset => INTERVAL '90 days', end_offset => INTERVAL '1 hour',
  schedule_interval => INTERVAL '1 hour', if_not_exists => true);

-- Router: pick the coarsest tier whose native bucket <= requested bucket, then
-- (re)bucket to p_bucket. Returns per-BMU rows (avg MW over time per bucket); the
-- dashboard panels sum across BMUs or break out per BMU as needed.
CREATE OR REPLACE FUNCTION dispatch_series(
    p_station text, p_from timestamp, p_to timestamp, p_bucket interval
) RETURNS TABLE(
    bucket_ts timestamp, bmu text, tier text, pn_mw double precision, boa_level_mw double precision,
    mel_mw double precision, so_turnup_mw double precision,
    boa_curtailment_mw double precision, so_curtailment_mw double precision
) LANGUAGE plpgsql STABLE AS $$
BEGIN
  IF p_bucket >= interval '1 day' THEN
    RETURN QUERY
      SELECT time_bucket(p_bucket, d.bucket)::timestamp, d.bmu, 'daily'::text,
             avg(d.pn_mw), avg(d.boa_level_mw), avg(d.mel_mw),
             avg(d.so_turnup_mw), avg(d.boa_curtailment_mw), avg(d.so_curtailment_mw)
      FROM bmu_dispatch_daily_mw d JOIN bmu b ON b.ngc_bm_unit = d.bmu
      WHERE b.station = p_station AND d.bucket >= p_from AND d.bucket < p_to
      GROUP BY 1, d.bmu ORDER BY 1, d.bmu;
  ELSIF p_bucket >= interval '1 hour' THEN
    RETURN QUERY
      SELECT time_bucket(p_bucket, d.bucket)::timestamp, d.bmu, 'hourly'::text,
             avg(d.pn_mw), avg(d.boa_level_mw), avg(d.mel_mw),
             avg(d.so_turnup_mw), avg(d.boa_curtailment_mw), avg(d.so_curtailment_mw)
      FROM bmu_dispatch_hourly d JOIN bmu b ON b.ngc_bm_unit = d.bmu
      WHERE b.station = p_station AND d.bucket >= p_from AND d.bucket < p_to
      GROUP BY 1, d.bmu ORDER BY 1, d.bmu;
  ELSE
    RETURN QUERY
      SELECT time_bucket(GREATEST(p_bucket, interval '5 min'), d.ts)::timestamp, d.bmu, 'raw'::text,
             avg(d.pn_mw), avg(d.boa_level_mw), avg(d.mel_mw),
             avg(d.so_turnup_mw), avg(d.boa_curtailment_mw), avg(d.so_curtailment_mw)
      FROM bmu_dispatch d JOIN bmu b ON b.ngc_bm_unit = d.bmu
      WHERE b.station = p_station AND d.ts >= p_from AND d.ts < p_to
      GROUP BY 1, d.bmu ORDER BY 1, d.bmu;
  END IF;
END $$;
