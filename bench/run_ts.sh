#!/usr/bin/env bash
# Set up the gb_grid_ts TimescaleDB clone and report status.
# Run inside `nix develop`. Idempotent: drops & recreates the clone each time.
set -euo pipefail

PGSOCK="$PWD/.postgres"
SRC="$GB_GRID_DATABASE_URL"
TS="postgresql://localhost:5433/gb_grid_ts?host=$PGSOCK"

echo "--- restart postgres to load shared_preload_libraries ---"
pg_ctl -D "$PGDATA" restart -m fast -o "-k '$PGSOCK'" >/dev/null
sleep 1
echo -n "shared_preload_libraries = "
psql "$SRC" -tAc "SHOW shared_preload_libraries;"
echo -n "timescaledb available = "
psql "$SRC" -tAc "SELECT default_version FROM pg_available_extensions WHERE name = 'timescaledb';"

echo "--- (re)create clone gb_grid_ts ---"
psql "$SRC" -c "DROP DATABASE IF EXISTS gb_grid_ts;"
psql "$SRC" -c "CREATE DATABASE gb_grid_ts;"

echo "--- dump | restore ---"
time pg_dump "$SRC" --no-owner --no-privileges | psql -q "$TS"

echo "--- bmu_dispatch rows in clone ---"
psql "$TS" -tAc "SELECT count(*) FROM bmu_dispatch;"
echo "clone ready: $TS"
