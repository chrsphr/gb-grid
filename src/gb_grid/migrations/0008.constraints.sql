CREATE TABLE IF NOT EXISTS constraints (
    constraint_group  TEXT NOT NULL,
    ts                TIMESTAMP NOT NULL,  -- UTC
    limit_mw          DOUBLE PRECISION,
    flow_mw           DOUBLE PRECISION,
    PRIMARY KEY (constraint_group, ts)
);

CREATE INDEX IF NOT EXISTS constraints_ts_idx ON constraints (ts);
