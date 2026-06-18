-- Enable columnar compression on the large hypertables and compress every
-- existing chunk. In production a compression policy would compress only chunks
-- older than the revision window (e.g. 30 days), leaving hot chunks row-store.
ALTER TABLE bmu_dispatch SET (timescaledb.compress,
      timescaledb.compress_segmentby = 'bmu', timescaledb.compress_orderby = 'ts');
ALTER TABLE pn SET (timescaledb.compress,
      timescaledb.compress_segmentby = 'national_grid_bm_unit', timescaledb.compress_orderby = 'time_from');
ALTER TABLE mels SET (timescaledb.compress,
      timescaledb.compress_segmentby = 'national_grid_bm_unit', timescaledb.compress_orderby = 'time_from');
ALTER TABLE b1610 SET (timescaledb.compress,
      timescaledb.compress_segmentby = 'ngc_bm_unit', timescaledb.compress_orderby = 'settlement_period');
ALTER TABLE constraints SET (timescaledb.compress,
      timescaledb.compress_segmentby = 'constraint_group', timescaledb.compress_orderby = 'ts');
ALTER TABLE boalf SET (timescaledb.compress,
      timescaledb.compress_segmentby = 'bm_unit', timescaledb.compress_orderby = 'time_from');

SELECT compress_chunk(c) FROM show_chunks('bmu_dispatch') c;
SELECT compress_chunk(c) FROM show_chunks('pn') c;
SELECT compress_chunk(c) FROM show_chunks('mels') c;
SELECT compress_chunk(c) FROM show_chunks('b1610') c;
SELECT compress_chunk(c) FROM show_chunks('constraints') c;
SELECT compress_chunk(c) FROM show_chunks('boalf') c;
