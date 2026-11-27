#!/usr/bin/env bash
# Full-pipeline golden diff: Python `gb-grid` vs the Rust `gb-grid-ingest`,
# each run end to end against the LIVE Elexon API into its own scratch DB:
#
#   backfill (all datasets) -> materialize-dispatch -> materialize-dispatch-daily
#
# then every ingested table, the watermarks, bmu_dispatch, bmu_dispatch_daily
# and every continuous aggregate are dumped and diffed. Unlike
# golden_materialize.sh (which copies a slice of the dev DB), this exercises the
# fetch/parse/upsert path too, so it can be pointed at windows the dev DB has
# never held — winter (GMT) and the BST clock changes in particular.
#
# It also prints a clock-change report: settlement periods per settlement date
# (expect 46 on spring-forward, 50 on fall-back, 48 otherwise) and 5-min grid
# points per UTC day in bmu_dispatch (always 288 — the grid is UTC).
#
# Requirements: inside `nix develop` (psql, .venv, GB_GRID_DATABASE_URL), dev
# Postgres up, and ingester/target/release/gb-grid-ingest built.
#
# Usage:
#   bash bench/rs_vs_py/golden_pipeline.sh --from 2025-10-25 --to 2025-10-27 [--workers 4] [--keep]
#
# Exits non-zero if anything differs. --keep leaves the scratch DBs for poking at.

set -euo pipefail

FROM=""; TO=""; WORKERS=4; KEEP=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --from) FROM="$2"; shift 2 ;;
    --to) TO="$2"; shift 2 ;;
    --workers) WORKERS="$2"; shift 2 ;;
    --keep) KEEP=1; shift ;;
    *) echo "unknown arg: $1" >&2; exit 1 ;;
  esac
done
[[ -n "$FROM" && -n "$TO" ]] || { echo "need --from and --to" >&2; exit 1; }
[[ -n "${GB_GRID_DATABASE_URL:-}" ]] || { echo "run inside \`nix develop\`" >&2; exit 1; }

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PY_BIN="$ROOT/.venv/bin/gb-grid"
RS_BIN="$ROOT/ingester/target/release/gb-grid-ingest"
BASE="postgresql://gb_grid:gbgrid@127.0.0.1:5433"
TAG="${FROM//-/}_$$"
PY_DB="gb_grid_gp_py_$TAG"
RS_DB="gb_grid_gp_rs_$TAG"
WORK="$(mktemp -d)"

cleanup() {
  rm -rf "$WORK"
  if [[ "$KEEP" == 0 ]]; then
    psql "$BASE/postgres" -q -c "DROP DATABASE IF EXISTS $PY_DB WITH (FORCE)" || true
    psql "$BASE/postgres" -q -c "DROP DATABASE IF EXISTS $RS_DB WITH (FORCE)" || true
  else
    echo "kept scratch DBs: $PY_DB $RS_DB"
  fi
}
trap cleanup EXIT

for f in "$PY_BIN" "$RS_BIN" psql; do
  command -v "$f" >/dev/null 2>&1 || { echo "missing dependency: $f" >&2; exit 1; }
done

# numpy/pandas wheels need libstdc++/zlib on NixOS (see golden_materialize.sh).
CC_LIB="$(nix build nixpkgs#stdenv.cc.cc.lib --no-link --print-out-paths 2>/dev/null || true)"
ZLIB="$(nix build nixpkgs#zlib --no-link --print-out-paths 2>/dev/null || true)"
export LD_LIBRARY_PATH="${CC_LIB:+$CC_LIB/lib:}${ZLIB:+$ZLIB/lib}${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"

say() { echo "== $*"; }
q() { psql "$BASE/$1" -q -At -F $'\t' -c "$2"; }

measure() {  # label cmd... ; wall/user to $WORK/<label>.time, aborts on failure
  local label="$1"; shift
  local TIMEFORMAT=$'real %R\nuser %U'
  if ! { time "$@" >"$WORK/$label.out" 2>"$WORK/$label.err"; } 2>"$WORK/$label.time"; then
    echo "FAILED: $label"; tail -n 30 "$WORK/$label.out" "$WORK/$label.err"; exit 1
  fi
}
t() { awk -v k="$2" '$1==k{print $2}' "$WORK/$1.time"; }

run_both() {  # label subcommand args...
  local label="$1"; shift
  say "$label: python"
  measure "py_$label" env GB_GRID_DATABASE_URL="$BASE/$PY_DB" "$PY_BIN" "$@"
  say "$label: rust"
  measure "rs_$label" env GB_GRID_DATABASE_URL="$BASE/$RS_DB" "$RS_BIN" "$@"
}

say "scratch DBs for $FROM..$TO: $PY_DB / $RS_DB"
for db in "$PY_DB" "$RS_DB"; do
  psql "$BASE/postgres" -q -c "DROP DATABASE IF EXISTS $db WITH (FORCE)"
  psql "$BASE/postgres" -q -c "CREATE DATABASE $db"
  GB_GRID_DATABASE_URL="$BASE/$db" "$PY_BIN" migrate >/dev/null
done

run_both backfill backfill --from "$FROM" --to "$TO"
run_both materialize materialize-dispatch --from "$FROM" --to "$TO" --workers "$WORKERS"
run_both daily materialize-dispatch-daily --from "$FROM" --to "$TO"

# ---- diff -------------------------------------------------------------------
# Rows are sorted textually (C locale) so dump order never matters. `-0` -> `0`:
# numpy can emit a signed zero where Rust emits +0.0; numerically identical.
dump() {  # db sql outfile
  q "$1" "$2" | awk 'BEGIN{FS=OFS="\t"} {for(i=1;i<=NF;i++) if($i=="-0") $i="0"; print}' \
    | LC_ALL=C sort > "$3"
}

declare -A SQL=(
  [fuelinst]="SELECT * FROM fuelinst"
  [boalf]="SELECT * FROM boalf"
  [b1610]="SELECT * FROM b1610"
  [pn]="SELECT * FROM pn"
  [mels]="SELECT * FROM mels"
  [system_prices]="SELECT * FROM system_prices"
  [ingest_watermark]="SELECT dataset, last_ts FROM ingest_watermark"
  [bmu_dispatch]="SELECT * FROM bmu_dispatch"
  [bmu_dispatch_daily]="SELECT * FROM bmu_dispatch_daily"
)
ORDER=(fuelinst boalf b1610 pn mels system_prices ingest_watermark bmu_dispatch bmu_dispatch_daily)
declare -A CAGG=()
for v in $(q "$RS_DB" "SELECT view_name FROM timescaledb_information.continuous_aggregates ORDER BY 1"); do
  SQL[$v]="SELECT * FROM $v"; ORDER+=("$v"); CAGG[$v]=1
done
# bmu_dispatch_daily is materialised from bmu_dispatch_daily_cagg, so it
# inherits the same float-summation-order noise.
CAGG[bmu_dispatch_daily]=1

# Continuous aggregates AVG/SUM floats in the table's physical row order, which
# differs between the two DBs (rows are written in worker-completion order), so
# their last bits can differ even when the source rows are bit-identical. For
# CAGGs only, fall back to: same rows/keys/text fields, floats within 1e-12
# relative. Base tables must still match exactly.
near_equal() {  # file_a file_b -> exit 0 if equal within tolerance
  [[ $(wc -l < "$1") == $(wc -l < "$2") ]] || return 1
  paste "$1" "$2" | awk -F'\t' '
    function abs(x) { return x < 0 ? -x : x }
    { n = NF / 2
      for (i = 1; i <= n; i++) {
        a = $i; b = $(i + n)
        if (a == b) continue
        if (a ~ /^-?[0-9.]+(e[-+]?[0-9]+)?$/ && b ~ /^-?[0-9.]+(e[-+]?[0-9]+)?$/) {
          m = abs(a) > abs(b) ? abs(a) : abs(b); if (m < 1) m = 1
          if (abs(a - b) / m <= 1e-12) continue
        }
        exit 1
      } }'
}

say "diffing ${#ORDER[@]} relations"
FAIL=0
printf '  %-26s %10s %10s  %s\n' relation python rust result
for rel in "${ORDER[@]}"; do
  dump "$PY_DB" "${SQL[$rel]}" "$WORK/py.$rel"
  dump "$RS_DB" "${SQL[$rel]}" "$WORK/rs.$rel"
  np=$(wc -l < "$WORK/py.$rel"); nr=$(wc -l < "$WORK/rs.$rel")
  if cmp -s "$WORK/py.$rel" "$WORK/rs.$rel"; then res=identical
  elif [[ -n "${CAGG[$rel]:-}" ]] && near_equal "$WORK/py.$rel" "$WORK/rs.$rel"; then
    res="equal (<=1e-12; float sum order)"
  else res=DIFFERS; FAIL=1; fi
  printf '  %-26s %10s %10s  %s\n' "$rel" "$np" "$nr" "$res"
  if [[ "$res" == DIFFERS ]]; then
    { diff "$WORK/py.$rel" "$WORK/rs.$rel" || true; } | head -12 | sed 's/^/      /'
  fi
done

# ---- clock-change report (rust DB; the diff above says whether python agrees) ---
say "settlement periods per settlement_date (n distinct / max)"
q "$RS_DB" "
  WITH d AS (SELECT generate_series('$FROM'::date, '$TO'::date, '1 day')::date AS sd),
  per AS (
    SELECT 'b1610' t, settlement_date sd, count(DISTINCT settlement_period) n, max(settlement_period) mx FROM b1610 GROUP BY 2
    UNION ALL SELECT 'system_prices', settlement_date, count(DISTINCT settlement_period), max(settlement_period) FROM system_prices GROUP BY 2
    UNION ALL SELECT 'fuelinst', settlement_date, count(DISTINCT settlement_period), max(settlement_period) FROM fuelinst GROUP BY 2
    UNION ALL SELECT 'pn', settlement_date, count(DISTINCT settlement_period), max(settlement_period) FROM pn GROUP BY 2
    UNION ALL SELECT 'mels', settlement_date, count(DISTINCT settlement_period), max(settlement_period) FROM mels GROUP BY 2
  )
  SELECT d.sd,
         ((d.sd + 1)::timestamp AT TIME ZONE 'Europe/London' - d.sd::timestamp AT TIME ZONE 'Europe/London') AS local_day_len,
         string_agg(per.t || '=' || per.n || '/' || per.mx, '  ' ORDER BY per.t)
  FROM d LEFT JOIN per USING (sd) GROUP BY d.sd ORDER BY d.sd" | sed 's/^/  /'

say "bmu_dispatch 5-min grid points per UTC day (per BMU: min / max; expect 288)"
q "$RS_DB" "
  SELECT day, min(n), max(n), count(*) AS bmus FROM (
    SELECT bmu, ts::date AS day, count(*) n FROM bmu_dispatch
    WHERE ts >= '$FROM' AND ts < '$TO'::date + 1 GROUP BY 1, 2) x
  GROUP BY day ORDER BY day" | sed 's/^/  /'

echo
echo "timings ($FROM..$TO):"
printf '  %-12s %10s %10s %10s %10s\n' step py_wall rs_wall py_user rs_user
for s in backfill materialize daily; do
  printf '  %-12s %10s %10s %10s %10s\n' "$s" "$(t py_$s real)" "$(t rs_$s real)" "$(t py_$s user)" "$(t rs_$s user)"
done

if [[ "$FAIL" == 0 ]]; then echo "GOLDEN PIPELINE: IDENTICAL"; else echo "GOLDEN PIPELINE: DIFFERS"; exit 1; fi
