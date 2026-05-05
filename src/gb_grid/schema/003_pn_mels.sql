CREATE TABLE IF NOT EXISTS pn (
    national_grid_bm_unit TEXT NOT NULL,
    bm_unit               TEXT,
    settlement_date       DATE,
    settlement_period     SMALLINT,
    time_from             TIMESTAMP NOT NULL,
    time_to               TIMESTAMP,
    level_from            DOUBLE,
    level_to              DOUBLE,
    PRIMARY KEY (national_grid_bm_unit, time_from)
);

CREATE INDEX IF NOT EXISTS pn_time_from_idx ON pn (time_from);
CREATE INDEX IF NOT EXISTS pn_ngc_idx ON pn (national_grid_bm_unit);

CREATE TABLE IF NOT EXISTS mels (
    national_grid_bm_unit TEXT NOT NULL,
    bm_unit               TEXT,
    settlement_date       DATE,
    settlement_period     SMALLINT,
    time_from             TIMESTAMP NOT NULL,
    time_to               TIMESTAMP,
    level_from            DOUBLE,
    level_to              DOUBLE,
    notification_time     TIMESTAMP,
    notification_sequence BIGINT NOT NULL,
    PRIMARY KEY (national_grid_bm_unit, time_from, notification_sequence)
);

CREATE INDEX IF NOT EXISTS mels_time_from_idx ON mels (time_from);
CREATE INDEX IF NOT EXISTS mels_ngc_idx ON mels (national_grid_bm_unit);
