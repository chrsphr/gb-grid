#!/usr/bin/env bash
# Golden diff: Python `materialize_dispatch` vs the Rust `gb-grid-ingest`
# binary, over the SAME real data, into two scratch databases. Also times both
# (wall + user) so the CPU win is measured, not assumed.
#
# Requirements (inside `nix develop`, which has psql + .venv + cargo):
#   - dev Postgres up (docker compose), the compose DB holding real PN/BOALF/
#     MELS/B1610 data (the "source" the window is sliced from), and
#     GB_GRID_DATABASE_URL exported.
#   - a Rust release binary at ingester/target/release/gb-grid-ingest
#     (`cargo build --release` in ingester/).
#
# Usage:
#   bash bench/rs_vs_py/golden_materialize.sh [--from 2026-05-01] [--to 2026-05-01] [--workers 4]
#
# Exits non-zero if the two bmu_dispatch tables differ. Prints a timing table.

set -euo pipefail

FROM="${FROM:-2026-05-01}"
TO="${TO:-2026-05-01}"
WORKERS="${WORKERS:-4}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --from) FROM="$2"; shift 2 ;;
    --to) TO="$2"; shift 2 ;;
    --workers) WORKERS="$2"; shift 2 ;;
    *) echo "unknown arg: $1" >&2; exit 1 ;;
  esac
done

if [[ -z "${GB_GRID_DATABASE_URL:-}" ]]; then
  echo "GB_GRID_DATABASE_URL not set — run inside \`nix develop\`." >&2
  exit 1
fi

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PY_BIN="$ROOT/.venv/bin/gb-grid"
RS_BIN="$ROOT/ingester/target/release/gb-grid-ingest"
ADMIN="postgresql://gb_grid:gbgrid@127.0.0.1:5433/postgres"
PID=$$
PY_DB="gb_grid_golden_py_$PID"
RS_DB="gb_grid_golden_rs_$PID"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

for f in "$PY_BIN" "$RS_BIN" psql; do
  command -v "$f" >/dev/null 2>&1 || { echo "missing dependency: $f" >&2; exit 1; }
done

# The venv's pandas/numpy wheels dlopen libstdc++ via NixOS store paths; the
# devShell exports LD_LIBRARY_PATH. Prepending the gcc/zlib libs here makes the
# script work even when an outer shell set LD_LIBRARY_PATH to something else
# (e.g. `nix shell nixpkgs#postgresql_16`).
CC_LIB="$(nix build nixpkgs#stdenv.cc.cc.lib --no-link --print-out-paths 2>/dev/null || true)"
ZLIB="$(nix build nixpkgs#zlib --no-link --print-out-paths 2>/dev/null || true)"
export LD_LIBRARY_PATH="${CC_LIB:+$CC_LIB/lib:}${ZLIB:+$ZLIB/lib}${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"

say() { echo "== $*"; }

# Time a command (wall/user/sys) with output going to $WORK/<label>.{out,err}
# and GNU-time-compatible stats to $WORK/<label>.time. Bash's `time` keyword
# measures the whole pipeline including children, and works on NixOS where
# /usr/bin/time is absent.
measure() {
  local label="$1"; shift
  local TIMEFORMAT=$'real %R\nuser %U\nsys %S'
  { time "$@" >"$WORK/$label.out" 2>"$WORK/$label.err"; } 2>"$WORK/$label.time"
}

recreate_db() {  # $1 = dbname
  psql "$ADMIN" -q -c "DROP DATABASE IF EXISTS $1 WITH (FORCE)"
  psql "$ADMIN" -q -c "CREATE DATABASE $1"
}

migrate_db() {  # $1 = dbname
  GB_GRID_DATABASE_URL="postgresql://gb_grid:gbgrid@127.0.0.1:5433/$1" "$PY_BIN" migrate >/dev/null
}

copy_slice() {  # $1 = dbname
  local url="postgresql://gb_grid:gbgrid@127.0.0.1:5433/$1"
  psql "$url" -q -c "TRUNCATE pn, boalf, mels, b1610, bmu_dispatch, bmu_dispatch_daily"
  psql "$GB_GRID_DATABASE_URL" -c "\copy (SELECT * FROM pn WHERE time_to >= '$FROM' AND time_from < '$TO'::date + 1) TO '$WORK/pn.csv' CSV"
  psql "$url" -c "\copy pn FROM '$WORK/pn.csv' CSV"
  psql "$GB_GRID_DATABASE_URL" -c "\copy (SELECT * FROM boalf WHERE time_to >= '$FROM' AND time_from < '$TO'::date + 1) TO '$WORK/boalf.csv' CSV"
  psql "$url" -c "\copy boalf FROM '$WORK/boalf.csv' CSV"
  psql "$GB_GRID_DATABASE_URL" -c "\copy (SELECT * FROM mels WHERE time_to >= '$FROM' AND time_from < '$TO'::date + 1) TO '$WORK/mels.csv' CSV"
  psql "$url" -c "\copy mels FROM '$WORK/mels.csv' CSV"
  psql "$GB_GRID_DATABASE_URL" -c "\copy (SELECT * FROM b1610 WHERE settlement_date >= '$FROM' AND settlement_date <= '$TO') TO '$WORK/b1610.csv' CSV"
  psql "$url" -c "\copy b1610 FROM '$WORK/b1610.csv' CSV"
}

dump_dispatch() {  # $1 = dbname, $2 = outfile
  # Normalise `-0` → `0`: Python stores -0.0 in so_curtailment_mw (numpy
  # clip-on-signed-zero) while Rust stores +0.0; they are numerically
  # identical, so the golden comparison ignores the sign of zero.
  psql "postgresql://gb_grid:gbgrid@127.0.0.1:5433/$1" -q -A -F $'\t' -c \
    "SELECT bmu, ts, pn_mw, boa_level_mw, mel_mw, so_turnup_mw, boa_curtailment_mw, so_curtailment_mw FROM bmu_dispatch ORDER BY bmu, ts" |
    awk 'BEGIN{FS=OFS="\t"} {for(i=1;i<=NF;i++) if($i=="-0") $i="0"; print}' > "$2"
}

say "scratch DBs ($FROM..$TO, workers=$WORKERS)"
recreate_db "$PY_DB"; recreate_db "$RS_DB"
migrate_db "$PY_DB"; migrate_db "$RS_DB"
say "copying real $FROM..$TO slice from the dev DB into both scratch DBs"
copy_slice "$PY_DB"; copy_slice "$RS_DB"

say "running Python materialize-dispatch"
measure py \
  env GB_GRID_DATABASE_URL="postgresql://gb_grid:gbgrid@127.0.0.1:5433/$PY_DB" \
  "$PY_BIN" materialize-dispatch --from "$FROM" --to "$TO" --workers "$WORKERS" || {
  echo "PYTHON MATERIALIZE FAILED"; cat "$WORK/py.out" "$WORK/py.err"; exit 1; }

say "running Rust materialize-dispatch"
measure rs \
  env GB_GRID_DATABASE_URL="postgresql://gb_grid:gbgrid@127.0.0.1:5433/$RS_DB" \
  "$RS_BIN" materialize-dispatch --from "$FROM" --to "$TO" --workers "$WORKERS" || {
  echo "RUST MATERIALIZE FAILED"; cat "$WORK/rs.out" "$WORK/rs.err"; exit 1; }

say "dumping and diffing bmu_dispatch"
dump_dispatch "$PY_DB" "$WORK/py.dump"
dump_dispatch "$RS_DB" "$WORK/rs.dump"

PY_ROWS="$(wc -l < "$WORK/py.dump")"
RS_ROWS="$(wc -l < "$WORK/rs.dump")"
echo "python rows: $PY_ROWS   rust rows: $RS_ROWS"

if diff -q "$WORK/py.dump" "$WORK/rs.dump" >/dev/null; then
  echo "GOLDEN DIFF: IDENTICAL"
  DIFF="IDENTICAL"
else
  echo "GOLDEN DIFF: DIFFERS"
  diff "$WORK/py.dump" "$WORK/rs.dump" | head -40
  DIFF="DIFFERS"
fi

extract_time() { grep "$1" "$2" | awk '{print $2}' | tr -d ' '; }
py_wall="$(extract_time 'real' "$WORK/py.time")"
rs_wall="$(extract_time 'real' "$WORK/rs.time")"
py_user="$(extract_time 'user' "$WORK/py.time")"
rs_user="$(extract_time 'user' "$WORK/rs.time")"

echo
echo "timings ($FROM..$TO, $WORKERS workers):"
printf '  %-8s %-12s %-12s\n' "" "python" "rust"
printf '  %-8s %-12s %-12s\n' "wall" "$py_wall" "$rs_wall"
printf '  %-8s %-12s %-12s\n' "user" "$py_user" "$rs_user"

[[ "$DIFF" == "IDENTICAL" ]] || exit 1