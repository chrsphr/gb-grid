ALTER TABLE boalf ADD COLUMN IF NOT EXISTS ngc_bm_unit TEXT;
CREATE INDEX IF NOT EXISTS boalf_ngc_idx ON boalf (ngc_bm_unit);
