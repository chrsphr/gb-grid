-- B1610's `quantity` field is energy in MWh delivered over the 30-minute
-- settlement period, not instantaneous MW. Rename so the column matches what
-- the Elexon API returns; conversion to MW (×2) happens at the analytics
-- layer when overlaying against PN/BOA.

ALTER TABLE b1610 RENAME COLUMN quantity_mw TO quantity_mwh;
