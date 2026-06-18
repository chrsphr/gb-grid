-- Multi-resolution B1610 (actual generation) serving for the dispatch dashboard.
-- The B1610 line previously scanned raw half-hourly b1610 with a computed-timestamp
-- filter (no chunk exclusion, no downsampling) -> ~4s at year zoom. b1610_series()
-- mirrors dispatch_series(): daily tier reuses b1610_daily_cagg (migration 0009),
-- raw tier adds settlement_date chunk exclusion. Plain function (no CAGG/policy),
-- so it runs transactionally.

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
    -- half-hourly actual MW (quantity_mwh * 2), chunk-excluded on settlement_date
    RETURN QUERY
      SELECT t.bt, 'raw'::text, SUM(t.mw)
      FROM (
        SELECT (a.settlement_date::timestamp + ((a.settlement_period - 1) * interval '30 min'))::timestamp AS bt,
               a.quantity_mwh * 2 AS mw
        FROM b1610 a JOIN bmu b ON b.ngc_bm_unit = a.ngc_bm_unit
        WHERE b.station = p_station
          AND a.settlement_date >= p_from::date AND a.settlement_date <= p_to::date
          AND (a.settlement_date::timestamp + ((a.settlement_period - 1) * interval '30 min')) >= p_from
          AND (a.settlement_date::timestamp + ((a.settlement_period - 1) * interval '30 min')) <  p_to
      ) t
      GROUP BY t.bt ORDER BY t.bt;
  END IF;
END $$;
