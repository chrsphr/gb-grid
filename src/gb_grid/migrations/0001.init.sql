CREATE TABLE IF NOT EXISTS fuelinst (
    publish_time      TIMESTAMP NOT NULL,
    settlement_date   DATE,
    settlement_period SMALLINT,
    fuel_type         TEXT NOT NULL,
    generation_mw     DOUBLE PRECISION,
    PRIMARY KEY (publish_time, fuel_type)
);

CREATE INDEX IF NOT EXISTS fuelinst_publish_time_idx ON fuelinst (publish_time);

CREATE TABLE IF NOT EXISTS b1610 (
    settlement_date   DATE NOT NULL,
    settlement_period SMALLINT NOT NULL,
    bm_unit           TEXT NOT NULL,
    ngc_bm_unit       TEXT,
    quantity_mw       DOUBLE PRECISION,
    PRIMARY KEY (settlement_date, settlement_period, bm_unit)
);

CREATE INDEX IF NOT EXISTS b1610_settlement_idx ON b1610 (settlement_date, settlement_period);

CREATE TABLE IF NOT EXISTS boalf (
    acceptance_id     BIGINT NOT NULL,
    bm_unit           TEXT NOT NULL,
    acceptance_time   TIMESTAMP,
    time_from         TIMESTAMP NOT NULL,
    time_to           TIMESTAMP,
    level_from        DOUBLE PRECISION,
    level_to          DOUBLE PRECISION,
    deemed_bo_flag    BOOLEAN,
    so_flag           BOOLEAN,
    ngc_bm_unit       TEXT,
    PRIMARY KEY (acceptance_id, time_from)
);

CREATE INDEX IF NOT EXISTS boalf_time_from_idx ON boalf (time_from);
CREATE INDEX IF NOT EXISTS boalf_bm_unit_idx ON boalf (bm_unit);
CREATE INDEX IF NOT EXISTS boalf_ngc_idx ON boalf (ngc_bm_unit);

CREATE TABLE IF NOT EXISTS system_prices (
    settlement_date       DATE NOT NULL,
    settlement_period     SMALLINT NOT NULL,
    system_sell_price     DOUBLE PRECISION,
    system_buy_price      DOUBLE PRECISION,
    net_imbalance_volume  DOUBLE PRECISION,
    PRIMARY KEY (settlement_date, settlement_period)
);

CREATE TABLE IF NOT EXISTS ingest_watermark (
    dataset    TEXT PRIMARY KEY,
    last_ts    TIMESTAMP,
    updated_at TIMESTAMP
);

CREATE TABLE IF NOT EXISTS pn (
    national_grid_bm_unit TEXT NOT NULL,
    bm_unit               TEXT,
    settlement_date       DATE,
    settlement_period     SMALLINT,
    time_from             TIMESTAMP NOT NULL,
    time_to               TIMESTAMP,
    level_from            DOUBLE PRECISION,
    level_to              DOUBLE PRECISION,
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
    level_from            DOUBLE PRECISION,
    level_to              DOUBLE PRECISION,
    notification_time     TIMESTAMP,
    notification_sequence BIGINT NOT NULL,
    PRIMARY KEY (national_grid_bm_unit, time_from, notification_sequence)
);

CREATE INDEX IF NOT EXISTS mels_time_from_idx ON mels (time_from);
CREATE INDEX IF NOT EXISTS mels_ngc_idx ON mels (national_grid_bm_unit);
