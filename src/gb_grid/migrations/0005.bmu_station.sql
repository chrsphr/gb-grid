-- Group BMUs into power stations by stripping the trailing unit-number suffix.
-- TORN-1, TORN-2  -> TORN; DRAXX1..6 -> DRAXX; AG-GSTK1.. -> AG-GSTK; SGRWO-1..6 -> SGRWO.
-- Single-BMU "stations" are just themselves.

ALTER TABLE bmu
    ADD COLUMN IF NOT EXISTS station TEXT
        GENERATED ALWAYS AS (regexp_replace(ngc_bm_unit, '-?\d+[A-Z]?$', '')) STORED;

CREATE INDEX IF NOT EXISTS bmu_station_idx ON bmu (station);
