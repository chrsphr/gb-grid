-- Elexon publishes BOALF rows for some BMUs (e.g. CUXTB-1) without a bmUnit
-- string — only the nationalGridBmUnit. ngc_bm_unit is the join key everywhere
-- in this DB, so let bm_unit be nullable rather than dropping real dispatch data.
ALTER TABLE boalf ALTER COLUMN bm_unit DROP NOT NULL;
