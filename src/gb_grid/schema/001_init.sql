CREATE TABLE IF NOT EXISTS fuelinst (
    publish_time      TIMESTAMP NOT NULL,
    settlement_date   DATE,
    settlement_period SMALLINT,
    fuel_type         TEXT NOT NULL,
    generation_mw     DOUBLE,
    PRIMARY KEY (publish_time, fuel_type)
);

CREATE INDEX IF NOT EXISTS fuelinst_publish_time_idx ON fuelinst (publish_time);

CREATE TABLE IF NOT EXISTS b1610 (
    settlement_date   DATE NOT NULL,
    settlement_period SMALLINT NOT NULL,
    bm_unit           TEXT NOT NULL,
    ngc_bm_unit       TEXT,
    quantity_mw       DOUBLE,
    PRIMARY KEY (settlement_date, settlement_period, bm_unit)
);

CREATE INDEX IF NOT EXISTS b1610_settlement_idx ON b1610 (settlement_date, settlement_period);

CREATE TABLE IF NOT EXISTS boalf (
    acceptance_id     BIGINT NOT NULL,
    bm_unit           TEXT NOT NULL,
    acceptance_time   TIMESTAMP,
    time_from         TIMESTAMP NOT NULL,
    time_to           TIMESTAMP,
    level_from        DOUBLE,
    level_to          DOUBLE,
    deemed_bo_flag    BOOLEAN,
    so_flag           BOOLEAN,
    PRIMARY KEY (acceptance_id, time_from)
);

CREATE INDEX IF NOT EXISTS boalf_time_from_idx ON boalf (time_from);
CREATE INDEX IF NOT EXISTS boalf_bm_unit_idx ON boalf (bm_unit);

CREATE TABLE IF NOT EXISTS system_prices (
    settlement_date       DATE NOT NULL,
    settlement_period     SMALLINT NOT NULL,
    system_sell_price     DOUBLE,
    system_buy_price      DOUBLE,
    net_imbalance_volume  DOUBLE,
    PRIMARY KEY (settlement_date, settlement_period)
);
