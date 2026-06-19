-- Fix the B1610 sub-daily time axis: settlement periods are anchored to UK local
-- (Europe/London) midnight, not UTC. The raw tier of b1610_series() previously
-- built the timestamp as `settlement_date 00:00 + (SP-1)*30min` and treated it as
-- naive UTC, so during BST every B1610 point landed an hour later than the
-- UTC-based dispatched/PN/MEL series it's overlaid on. Convert the local
-- wall-clock time through Europe/London -> UTC instead.
--
-- Daily tier is unchanged: it aggregates by UK settlement day, which is the
-- natural grain, and an intra-day hour is immaterial at day zoom.

CREATE OR REPLACE FUNCTION b1610_series(
    p_station text, p_from timestamp, p_to timestamp, p_bucket interval
) RETURNS TABLE(bucket_ts timestamp, tier text, b1610_mw double precision)
LANGUAGE plpgsql STABLE AS $$
BEGIN
  IF p_bucket >= interval '1 day' THEN
    -- daily average MW = daily energy (MWh) / 24h, summed across the station's BMUs
    RETURN QUERY
      SELECT d.bucket::timestamp, 'daily'::text, SUM(d.b1610_mwh) / 24.0
      FROM b1610_daily_cagg d JOIN bmu b ON b.ngc_bm_unit = d.bmu
      WHERE b.station = p_station AND d.bucket >= p_from::date AND d.bucket <= p_to::date
      GROUP BY d.bucket ORDER BY d.bucket;
  ELSE
    -- half-hourly actual MW (quantity_mwh * 2), chunk-excluded on settlement_date.
    -- The half-hour start is UK local; convert to UTC so it aligns with the
    -- UTC dispatch series.
    RETURN QUERY
      SELECT t.bt, 'raw'::text, SUM(t.mw)
      FROM (
        SELECT ((a.settlement_date::timestamp + ((a.settlement_period - 1) * interval '30 min'))
                  AT TIME ZONE 'Europe/London' AT TIME ZONE 'UTC')::timestamp AS bt,
               a.quantity_mwh * 2 AS mw
        FROM b1610 a JOIN bmu b ON b.ngc_bm_unit = a.ngc_bm_unit
        WHERE b.station = p_station
          AND a.settlement_date >= p_from::date AND a.settlement_date <= p_to::date
          AND ((a.settlement_date::timestamp + ((a.settlement_period - 1) * interval '30 min'))
                  AT TIME ZONE 'Europe/London' AT TIME ZONE 'UTC') >= p_from
          AND ((a.settlement_date::timestamp + ((a.settlement_period - 1) * interval '30 min'))
                  AT TIME ZONE 'Europe/London' AT TIME ZONE 'UTC') <  p_to
      ) t
      GROUP BY t.bt ORDER BY t.bt;
  END IF;
END $$;
