#!/usr/bin/env bash
# Golden diff for `refresh-constraints`: Python vs Rust, each downloading the
# full live NESO CSV into its own scratch DB, then the `constraints` tables are
# dumped and diffed. The file spans several years, so this covers every clock
# change in it. A per-clock-change report follows: rows per group on each
# transition day vs the local day's half-hour count (46 spring / 50 autumn).
#
# Requirements: inside `nix develop`, dev Postgres up, release binary built.
# Usage: bash bench/rs_vs_py/golden_constraints.sh [--keep]

set -euo pipefail
KEEP=0; [[ "${1:-}" == "--keep" ]] && KEEP=1
[[ -n "${GB_GRID_DATABASE_URL:-}" ]] || { echo "run inside \`nix develop\`" >&2; exit 1; }

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PY_BIN="$ROOT/.venv/bin/gb-grid"
RS_BIN="$ROOT/ingester/target/release/gb-grid-ingest"
BASE="postgresql://gb_grid:gbgrid@127.0.0.1:5433"
PY_DB="gb_grid_gc_py_$$"; RS_DB="gb_grid_gc_rs_$$"
WORK="$(mktemp -d)"
cleanup() {
  rm -rf "$WORK"
  if [[ "$KEEP" == 0 ]]; then
    for db in "$PY_DB" "$RS_DB"; do psql "$BASE/postgres" -q -c "DROP DATABASE IF EXISTS $db WITH (FORCE)" || true; done
  else echo "kept scratch DBs: $PY_DB $RS_DB"; fi
}
trap cleanup EXIT

say() { echo "== $*"; }
q() { psql "$BASE/$1" -q -At -F $'\t' -c "$2"; }
measure() {
  local label="$1"; shift
  local TIMEFORMAT=$'real %R\nuser %U'
  if ! { time "$@" >"$WORK/$label.out" 2>"$WORK/$label.err"; } 2>"$WORK/$label.time"; then
    echo "FAILED: $label"; tail -n 30 "$WORK/$label.out" "$WORK/$label.err"; exit 1
  fi
}
t() { awk -v k="$2" '$1==k{print $2}' "$WORK/$1.time"; }

for db in "$PY_DB" "$RS_DB"; do
  psql "$BASE/postgres" -q -c "CREATE DATABASE $db"
  GB_GRID_DATABASE_URL="$BASE/$db" "$PY_BIN" migrate >/dev/null
done

say "refresh-constraints: python"
measure py env GB_GRID_DATABASE_URL="$BASE/$PY_DB" "$PY_BIN" refresh-constraints
say "refresh-constraints: rust"
measure rs env GB_GRID_DATABASE_URL="$BASE/$RS_DB" "$RS_BIN" refresh-constraints
echo "  python: $(tail -n 1 "$WORK/py.out")   rust: $(tail -n 1 "$WORK/rs.out")"

for side in py rs; do
  db=$([[ $side == py ]] && echo "$PY_DB" || echo "$RS_DB")
  q "$db" "SELECT * FROM constraints" | awk 'BEGIN{FS=OFS="\t"} {for(i=1;i<=NF;i++) if($i=="-0") $i="0"; print}' \
    | LC_ALL=C sort > "$WORK/$side.dump"
done
NP=$(wc -l < "$WORK/py.dump"); NR=$(wc -l < "$WORK/rs.dump")
FAIL=0
if cmp -s "$WORK/py.dump" "$WORK/rs.dump"; then RES=identical; else RES=DIFFERS; FAIL=1; fi
echo "  constraints: python $NP rows, rust $NR rows -> $RES"
[[ "$RES" == DIFFERS ]] && { { diff "$WORK/py.dump" "$WORK/rs.dump" || true; } | head -12 | sed 's/^/      /'; }

say "clock-change days: rows per group vs local half-hours (rust DB)"
# A transition day's UTC span is [local midnight, next local midnight); count the
# rows that land in it per group. Fewer than the day's half-hours = lost slots.
q "$RS_DB" "
  WITH days AS (
    SELECT d::date AS sd FROM generate_series(
      (SELECT min(ts)::date FROM constraints), (SELECT max(ts)::date FROM constraints), '1 day') d
    WHERE (((d::date + 1)::timestamp AT TIME ZONE 'Europe/London') - (d::date::timestamp AT TIME ZONE 'Europe/London'))
          <> interval '24 hours'),
  span AS (
    SELECT sd,
      (sd::timestamp AT TIME ZONE 'Europe/London' AT TIME ZONE 'UTC') AS lo,
      ((sd + 1)::timestamp AT TIME ZONE 'Europe/London' AT TIME ZONE 'UTC') AS hi FROM days),
  per AS (
    SELECT s.sd, c.constraint_group, count(*) n,
           EXTRACT(EPOCH FROM s.hi - s.lo) / 1800 AS expect
    FROM span s JOIN constraints c ON c.ts >= s.lo AND c.ts < s.hi
    GROUP BY 1, 2, s.hi, s.lo)
  SELECT sd, expect::int AS half_hours, count(*) AS groups,
         sum((n = expect)::int) AS complete, min(n) AS min_rows, max(n) AS max_rows
  FROM per GROUP BY sd, expect ORDER BY sd" | sed 's/^/  /'

echo
printf '  %-8s %10s %10s\n' "" python rust
printf '  %-8s %10s %10s\n' wall "$(t py real)" "$(t rs real)"
printf '  %-8s %10s %10s\n' user "$(t py user)" "$(t rs user)"
if [[ "$FAIL" == 0 ]]; then echo "GOLDEN CONSTRAINTS: IDENTICAL"; else echo "GOLDEN CONSTRAINTS: DIFFERS"; exit 1; fi
