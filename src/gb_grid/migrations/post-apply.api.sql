-- The web UI's read API: an `api` schema of read-only functions that PostgREST
-- exposes as /rpc/<name> endpoints (PostgREST is configured with
-- db-schemas=api, db-anon-role=web_anon).
--
-- This is a yoyo *post-apply hook*, not a numbered migration. `gb-grid migrate`
-- re-runs it on every invocation (see gb_grid.db.migrate), and it is never
-- recorded as applied, so unlike migrations it is meant to be edited in place.
-- It must stay idempotent: it drops and recreates the whole `api` schema in one
-- transaction.
--
-- Conventions:
--   * Functions are STABLE and SECURITY INVOKER; they run as web_anon, which
--     only has SELECT on `public`.
--   * Time bounds are timestamptz in, timestamptz (or epoch-ms arrays) out. The
--     tables store naive UTC, so convert with `AT TIME ZONE 'UTC'`.
--   * Parameter names that collide with a column (notably `station`) are
--     qualified with the function name inside SQL bodies: in a SQL function a
--     bare name resolves to the column first.
--   * Large time series come back as one row per series with parallel
--     `ts` (epoch ms) / value arrays, which keeps the JSON payload small.

DROP SCHEMA IF EXISTS api CASCADE;
CREATE SCHEMA api;
COMMENT ON SCHEMA api IS 'Read API for the gb-grid web UI (served by PostgREST).';

-- ---------------------------------------------------------------------------
-- Reference data
-- ---------------------------------------------------------------------------

-- Every station (BMU id minus its unit suffix): a display name, mean location
-- of its BMUs (NULL if none are located), modal fuel type, BMU count, and
-- whether any of its BMUs has dispatch data (for the dispatch page's station
-- picker). The name is the most common BMU name with its trailing unit number
-- removed ("Seagreen1 Offshore WF 3" -> "Seagreen1 Offshore WF"); BMU names
-- without a space are usually just codes, so they're ignored.
CREATE FUNCTION api.stations()
RETURNS TABLE (
  station text, name text, latitude double precision, longitude double precision,
  fuel_type text, bmus int, has_dispatch boolean
) LANGUAGE sql STABLE AS $$
  WITH dispatched AS (SELECT DISTINCT bmu FROM bmu_dispatch_daily_mw)
  SELECT b.station,
         mode() WITHIN GROUP (
           ORDER BY nullif(btrim(regexp_replace(b.bmu_name, '[\s_-]*\d+[A-Z]?$', '')), ''))
           FILTER (WHERE b.bmu_name ~ '\s'),
         avg(b.latitude),
         avg(b.longitude),
         mode() WITHIN GROUP (ORDER BY coalesce(b.bmrs_fuel_type, 'UNKNOWN')),
         count(*)::int,
         bool_or(d.bmu IS NOT NULL)
  FROM bmu b
  LEFT JOIN dispatched d ON d.bmu = b.ngc_bm_unit
  WHERE b.station IS NOT NULL AND b.station <> ''
  GROUP BY b.station
  ORDER BY b.station
$$;

CREATE FUNCTION api.fuel_types()
RETURNS TABLE (fuel_type text) LANGUAGE sql STABLE AS $$
  SELECT DISTINCT coalesce(bmrs_fuel_type, 'UNKNOWN') FROM bmu ORDER BY 1
$$;

-- ---------------------------------------------------------------------------
-- BMU dispatch page
-- ---------------------------------------------------------------------------

-- Per-BMU dispatch series for one station at a requested bucket width. Wraps
-- public.dispatch_series, which picks the raw / hourly / daily tier from `step`.
CREATE FUNCTION api.dispatch_series(
  station text, start_ts timestamptz, end_ts timestamptz, step interval
) RETURNS TABLE (
  ts timestamptz, bmu text, tier text,
  pn_mw double precision, boa_level_mw double precision, mel_mw double precision,
  so_turnup_mw double precision, boa_curtailment_mw double precision,
  so_curtailment_mw double precision
) LANGUAGE sql STABLE AS $$
  SELECT s.bucket_ts AT TIME ZONE 'UTC', s.bmu, s.tier,
         s.pn_mw, s.boa_level_mw, s.mel_mw,
         s.so_turnup_mw, s.boa_curtailment_mw, s.so_curtailment_mw
  FROM public.dispatch_series(
         station, start_ts AT TIME ZONE 'UTC', end_ts AT TIME ZONE 'UTC', step) s
  ORDER BY 1, 2
$$;

-- Station-total B1610 metered output (MW). Half-hourly below a 1-day step,
-- daily averages at or above it.
CREATE FUNCTION api.b1610_series(
  station text, start_ts timestamptz, end_ts timestamptz, step interval
) RETURNS TABLE (ts timestamptz, tier text, b1610_mw double precision)
LANGUAGE sql STABLE AS $$
  SELECT s.bucket_ts AT TIME ZONE 'UTC', s.tier, s.b1610_mw
  FROM public.b1610_series(
         station, start_ts AT TIME ZONE 'UTC', end_ts AT TIME ZONE 'UTC', step) s
  ORDER BY 1
$$;

-- Energy totals for the dispatch page's stat tiles. Matches the Grafana
-- panels: whole UTC days from bmu_dispatch_daily whose 00:00 falls inside
-- [start_ts, end_ts].
CREATE FUNCTION api.dispatch_totals(
  station text, start_ts timestamptz, end_ts timestamptz
) RETURNS TABLE (
  pn_mwh double precision, b1610_mwh double precision,
  turnup_mwh double precision, turndown_mwh double precision,
  pn_minus_b1610_mwh double precision
) LANGUAGE sql STABLE AS $$
  SELECT sum(d.pn_mwh),
         sum(d.b1610_mwh),
         sum(d.so_turnup_mwh),
         sum(d.boa_curtailment_mwh),
         sum(d.pn_mwh) - sum(coalesce(d.b1610_mwh, 0))
  FROM bmu_dispatch_daily d
  JOIN bmu b ON b.ngc_bm_unit = d.bmu
  WHERE b.station = dispatch_totals.station
    AND d.date BETWEEN (start_ts AT TIME ZONE 'UTC') AND (end_ts AT TIME ZONE 'UTC')
$$;

-- ---------------------------------------------------------------------------
-- Annual summary page
-- ---------------------------------------------------------------------------

-- Daily energy per metric across a selection of stations and fuel types (NULL
-- array = no filter). With pct, each value is a % of the selection's capacity:
-- the sum over the selected BMUs of each BMU's peak MEL in the window, times
-- 24h (the Grafana dashboard's definition).
CREATE FUNCTION api.daily_calendar(
  start_ts timestamptz, end_ts timestamptz,
  fuels text[] DEFAULT NULL, stations text[] DEFAULT NULL, pct boolean DEFAULT false
) RETURNS TABLE (
  day date, pn double precision, dispatched double precision,
  b1610 double precision, so_turnup double precision,
  boa_curtailment double precision
) LANGUAGE plpgsql STABLE AS $$
#variable_conflict use_column
DECLARE
  t0 timestamp := start_ts AT TIME ZONE 'UTC';
  t1 timestamp := end_ts AT TIME ZONE 'UTC';
  scale double precision := 1.0;
BEGIN
  IF pct THEN
    SELECT 100.0 / nullif(coalesce(sum(m.cap), 0) * 24.0, 0) INTO scale
    FROM (
      SELECT national_grid_bm_unit AS ngc, max(greatest(level_from, level_to)) AS cap
      FROM mels WHERE time_from >= t0 AND time_from < t1 GROUP BY 1
    ) m
    JOIN bmu b ON b.ngc_bm_unit = m.ngc
    WHERE (stations IS NULL OR b.station = ANY (stations))
      AND (fuels IS NULL OR coalesce(b.bmrs_fuel_type, 'UNKNOWN') = ANY (fuels));
  END IF;

  RETURN QUERY
    SELECT d.date,
           sum(d.pn_mwh) * scale,
           sum(d.boa_dispatched_mwh) * scale,
           sum(d.b1610_mwh) * scale,
           sum(d.so_turnup_mwh) * scale,
           sum(d.boa_curtailment_mwh) * scale
    FROM bmu_dispatch_daily d
    JOIN bmu b ON b.ngc_bm_unit = d.bmu
    WHERE (stations IS NULL OR b.station = ANY (stations))
      AND (fuels IS NULL OR coalesce(b.bmrs_fuel_type, 'UNKNOWN') = ANY (fuels))
      AND d.date BETWEEN t0 AND t1
    GROUP BY d.date
    ORDER BY d.date;
END $$;

-- ---------------------------------------------------------------------------
-- Output heatmap page
-- ---------------------------------------------------------------------------

-- National generation by fuel type (fuelinst), one row per fuel with parallel
-- epoch-ms / MW arrays. `step` averages into coarser buckets (5 min minimum,
-- the native cadence).
CREATE FUNCTION api.fuel_mix(
  start_ts timestamptz, end_ts timestamptz, step interval DEFAULT NULL
) RETURNS TABLE (fuel_type text, ts bigint[], mw double precision[])
LANGUAGE sql STABLE AS $$
  WITH b AS (
    SELECT f.fuel_type,
           time_bucket(greatest(coalesce(step, interval '5 min'), interval '5 min'),
                       f.publish_time) AS t,
           avg(f.generation_mw) AS mw
    FROM fuelinst f
    WHERE f.publish_time >= (start_ts AT TIME ZONE 'UTC')
      AND f.publish_time <  (end_ts AT TIME ZONE 'UTC')
    GROUP BY 1, 2
  )
  SELECT b.fuel_type,
         array_agg((extract(epoch FROM b.t) * 1000)::bigint ORDER BY b.t),
         array_agg(b.mw ORDER BY b.t)
  FROM b
  GROUP BY b.fuel_type
  ORDER BY b.fuel_type
$$;

-- Half-hourly B1610 output per located station of one fuel type, from
-- b1610_hh_cagg. Rows are ranked north to south (rank 1 = northernmost). With
-- pct, values are % of the station's capacity (sum of its BMUs' peak MEL in the
-- window; stations with no MEL in the window are dropped); otherwise net MW
-- (negative = import / charging). Zero becomes NULL and is left out of the
-- arrays, as the Grafana panels blanked it.
--
-- Settlement periods are anchored at UK local midnight, so a period's instant
-- is (local midnight) + (sp-1)*30min. This is correct on clock-change days
-- (46/50 periods), unlike the bucket + (sp-1)*30min form in b1610_series().
CREATE FUNCTION api.station_grid(
  fuel text, start_ts timestamptz, end_ts timestamptz, pct boolean DEFAULT true
) RETURNS TABLE (station text, rank int, ts bigint[], value double precision[])
LANGUAGE sql STABLE AS $$
  WITH sel AS (
    SELECT b.ngc_bm_unit, b.station, b.latitude
    FROM bmu b
    WHERE coalesce(b.bmrs_fuel_type, 'UNKNOWN') = station_grid.fuel
      AND b.latitude IS NOT NULL AND b.longitude IS NOT NULL
  ),
  ns AS (
    SELECT sel.station, row_number() OVER (ORDER BY avg(sel.latitude) DESC)::int AS rnk
    FROM sel GROUP BY sel.station
  ),
  cap AS (
    SELECT sel.station, sum(m.cap) AS cap
    FROM (
      SELECT national_grid_bm_unit AS ngc, max(greatest(level_from, level_to)) AS cap
      FROM mels
      WHERE time_from >= (start_ts AT TIME ZONE 'UTC')
        AND time_from <  (end_ts AT TIME ZONE 'UTC')
      GROUP BY 1
    ) m
    JOIN sel ON sel.ngc_bm_unit = m.ngc
    GROUP BY sel.station
  ),
  cells AS (
    SELECT sel.station,
           (h.bucket::timestamp AT TIME ZONE 'Europe/London')
             + (h.settlement_period - 1) * interval '30 min' AS t,
           sum(h.b1610_mwh * 2) AS mw
    FROM b1610_hh_cagg h
    JOIN sel ON sel.ngc_bm_unit = h.bmu
    WHERE h.bucket >= (start_ts AT TIME ZONE 'UTC')::date - 1
      AND h.bucket <= (end_ts AT TIME ZONE 'UTC')::date + 1
    GROUP BY 1, 2
  ),
  vals AS (
    SELECT c.station, c.t,
           nullif(CASE WHEN pct THEN 100.0 * c.mw / nullif(cap.cap, 0) ELSE c.mw END, 0) AS v
    FROM cells c
    LEFT JOIN cap ON cap.station = c.station
    WHERE c.t >= start_ts AND c.t < end_ts
      AND (NOT pct OR cap.station IS NOT NULL)
  )
  SELECT ns.station, ns.rnk,
         coalesce(array_agg((extract(epoch FROM v.t) * 1000)::bigint ORDER BY v.t)
                    FILTER (WHERE v.v IS NOT NULL), '{}'),
         coalesce(array_agg(v.v ORDER BY v.t) FILTER (WHERE v.v IS NOT NULL), '{}')
  FROM vals v
  JOIN ns ON ns.station = v.station
  GROUP BY ns.station, ns.rnk
  ORDER BY ns.rnk
$$;

-- ---------------------------------------------------------------------------
-- Constraints page
-- ---------------------------------------------------------------------------

-- The constraint groups shown on the constraints page, in display order, with
-- their boundary labels. Only groups with data in the window are returned.
CREATE FUNCTION api.constraint_groups(start_ts timestamptz, end_ts timestamptz)
RETURNS TABLE (ord int, label text, constraint_group text)
LANGUAGE sql STABLE AS $$
  SELECT t.ord, t.label, t.cg
  FROM (VALUES
    (1, 'ERROEX', 'ERROEX'), (2, 'B2 — SSEN-S', 'SSEN-S'), (3, 'B4 — SSE-SP2', 'SSE-SP2'),
    (4, 'B5 — NKILGRMO', 'NKILGRMO'), (5, 'GALLEX', 'GALLEX'), (6, 'B6 — SCOTEX', 'SCOTEX'),
    (7, 'SSE-SP', 'SSE-SP'), (8, 'FLOWSTH', 'FLOWSTH'), (9, 'HARSPNBLY', 'HARSPNBLY'),
    (10, 'B7 — SSHARN', 'SSHARN'), (11, 'B7 — SSHARN3', 'SSHARN3'), (12, 'SSHARN7', 'SSHARN7'),
    (13, 'SHARN', 'SHARN'), (14, 'SPANOREX', 'SPANOREX'), (15, 'GM+SNOW5A', 'GM+SNOW5A'),
    (16, 'GETEX', 'GETEX'), (17, 'BRASIZEX', 'BRASIZEX'), (18, 'SW1 — SWALEX', 'SWALEX'),
    (19, 'SEIMP', 'SEIMP'), (20, 'LE1 — SEIMPPR21', 'SEIMPPR21'),
    (21, 'LE1 — SEIMPPR23', 'SEIMPPR23'), (22, 'LE1 — SEIMPPR33', 'SEIMPPR33'),
    (23, 'B15 — ESTEX', 'ESTEX')
  ) AS t (ord, label, cg)
  WHERE EXISTS (
    SELECT 1 FROM constraints c
    WHERE c.constraint_group = t.cg
      AND c.ts BETWEEN (start_ts AT TIME ZONE 'UTC') AND (end_ts AT TIME ZONE 'UTC')
  )
  ORDER BY t.ord
$$;

-- Flow, limit, a 48-row (24h at half-hourly cadence) rolling mean of flow, and
-- each UTC day's flow max/min, per group, as parallel arrays.
CREATE FUNCTION api.constraint_series(
  groups text[], start_ts timestamptz, end_ts timestamptz
) RETURNS TABLE (
  constraint_group text, ts bigint[], flow_mw double precision[],
  avg_24h_mw double precision[], daily_max_mw double precision[],
  daily_min_mw double precision[], limit_mw double precision[]
) LANGUAGE sql STABLE AS $$
  WITH w AS (
    SELECT c.constraint_group, c.ts, c.flow_mw, c.limit_mw,
           avg(c.flow_mw) OVER (PARTITION BY c.constraint_group ORDER BY c.ts
                                ROWS BETWEEN 47 PRECEDING AND CURRENT ROW) AS avg24,
           max(c.flow_mw) OVER (PARTITION BY c.constraint_group, date_trunc('day', c.ts)) AS dmax,
           min(c.flow_mw) OVER (PARTITION BY c.constraint_group, date_trunc('day', c.ts)) AS dmin
    FROM constraints c
    WHERE c.constraint_group = ANY (groups)
      AND c.ts BETWEEN (start_ts AT TIME ZONE 'UTC') AND (end_ts AT TIME ZONE 'UTC')
  )
  SELECT w.constraint_group,
         array_agg((extract(epoch FROM w.ts) * 1000)::bigint ORDER BY w.ts),
         array_agg(w.flow_mw ORDER BY w.ts),
         array_agg(w.avg24 ORDER BY w.ts),
         array_agg(w.dmax ORDER BY w.ts),
         array_agg(w.dmin ORDER BY w.ts),
         array_agg(w.limit_mw ORDER BY w.ts)
  FROM w
  GROUP BY w.constraint_group
  ORDER BY w.constraint_group
$$;

-- ---------------------------------------------------------------------------
-- The web_anon role and its privileges. web_anon is PostgREST's anonymous
-- role. On NixOS PostgREST logs in as `postgrest` (peer auth) and switches to
-- web_anon; in docker compose it logs in as the gb_grid superuser, which can
-- switch to any role.
--
-- Creating the role needs CREATEROLE (the NixOS module grants it to gb_grid
-- when the web UI is enabled). Without it this only warns, so `migrate` never
-- blocks the ingester on a host that doesn't run the web UI.
--
-- Grants are re-applied on every run so newly added tables and continuous
-- aggregates are covered. PostgREST only exposes the `api` schema; SELECT on
-- `public` is what the SECURITY INVOKER functions above read through.
-- ---------------------------------------------------------------------------
DO $$
BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'web_anon') THEN
    BEGIN
      CREATE ROLE web_anon NOLOGIN;
    EXCEPTION WHEN insufficient_privilege THEN
      RAISE WARNING 'gb-grid: cannot create role web_anon (%); the web API needs it', SQLERRM;
      RETURN;
    END;
  END IF;

  BEGIN
    -- PostgREST applies these per-role settings to every request it runs.
    ALTER ROLE web_anon SET statement_timeout = '30s';
    ALTER ROLE web_anon SET timezone = 'UTC';
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'postgrest')
       AND NOT pg_has_role('postgrest', 'web_anon', 'MEMBER') THEN
      GRANT web_anon TO postgrest;
    END IF;
  EXCEPTION WHEN insufficient_privilege THEN
    RAISE WARNING 'gb-grid: cannot configure role web_anon (%)', SQLERRM;
  END;

  GRANT USAGE ON SCHEMA api, public TO web_anon;
  GRANT SELECT ON ALL TABLES IN SCHEMA public TO web_anon;
  GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA api TO web_anon;
END $$;

-- Tell a running PostgREST to reload its schema cache (delivered on commit).
NOTIFY pgrst, 'reload schema';
