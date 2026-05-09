CREATE TABLE IF NOT EXISTS bmu_dispatch (
    bmu                 TEXT NOT NULL,
    ts                  TIMESTAMP NOT NULL,
    pn_mw               DOUBLE PRECISION,
    boa_level_mw        DOUBLE PRECISION,
    mel_mw              DOUBLE PRECISION,
    so_turnup_mw        DOUBLE PRECISION,
    boa_curtailment_mw  DOUBLE PRECISION,
    so_curtailment_mw   DOUBLE PRECISION,
    PRIMARY KEY (bmu, ts)
);

CREATE INDEX IF NOT EXISTS bmu_dispatch_ts_idx ON bmu_dispatch (ts);
