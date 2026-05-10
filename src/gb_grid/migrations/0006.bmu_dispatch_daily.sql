-- Per-BMU per-day energy totals (MWh) for the annual-summary heatmap dashboard.
-- Aggregated from `bmu_dispatch` (5-min samples; MW * 5/60 -> MWh) and `b1610`.

CREATE TABLE IF NOT EXISTS bmu_dispatch_daily (
    bmu                  TEXT NOT NULL,
    date                 DATE NOT NULL,
    pn_mwh               DOUBLE PRECISION,
    boa_dispatched_mwh   DOUBLE PRECISION,
    so_turnup_mwh        DOUBLE PRECISION,
    boa_curtailment_mwh  DOUBLE PRECISION,
    so_curtailment_mwh   DOUBLE PRECISION,
    b1610_mwh            DOUBLE PRECISION,
    PRIMARY KEY (bmu, date)
);

CREATE INDEX IF NOT EXISTS bmu_dispatch_daily_date_idx ON bmu_dispatch_daily (date);
