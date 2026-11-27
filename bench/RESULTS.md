# TimescaleDB adoption — benchmark results

Dataset: ~2 weeks of dispatch (1–14 May 2026), 9.7M `bmu_dispatch` rows, 2.7 GB
total. `constraints` spans 6.6 years (951k rows). Methodology: clone `gb_grid`
→ `gb_grid_ts`, convert to hypertables + continuous aggregates, run identical
workload via `bench/bench.py`. Vanilla baseline left untouched. Median of 5.

## Headline

| Operation | Vanilla PG16 | TimescaleDB | Change |
|---|--:|--:|--:|
| **Daily rollup, trailing 14d** | 5620 ms | ~16 ms | **~350× faster** |
| **Total storage** | 2.7 GB | 114 MB | **23.8× smaller** |

Rollup: the vanilla scheduler brute-force recomputes 14 days every tick; the
continuous aggregate refresh only processes changed buckets (steady-state, no
new data = ~16 ms). This is the operational win.

## Production implementation (validated via `gb-grid migrate` on real dev DB)

Chosen design: keep `bmu_dispatch_daily` as an indexed **table**, refreshed from
the two CAGGs (FULL OUTER JOIN + upsert of ~35k rows) instead of scanning 9.7M
raw rows. Migration `0009.timescale.sql` applied cleanly: 7 hypertables, 2 CAGGs +
30-min refresh policies, 6 compression policies.

| Operation | Vanilla | Prod (CAGG-sourced table) | Change |
|---|--:|--:|--:|
| Daily rollup, 14d | 5620 ms | 436 ms | **~13× faster** |
| annual_summary read | 0.6 ms | 0.57 ms | unchanged (regression resolved) |

Rollup totals match baseline exactly (1730124.4 MWh). The remaining 436 ms is
mostly the 35k-row `executemany` upsert — the multi-row-VALUES upsert fix would
cut it further.

## Reads (hot / uncompressed chunks — the realistic dashboard case)

| Read query | Vanilla | TS (hot) | Note |
|---|--:|--:|---|
| dispatch, 1 station, 7d | 9.4 ms | 12.1 ms | ~flat (abs. tiny) |
| dispatch curtailment, 7d | 8.9 ms | 12.3 ms | ~flat |
| constraints, 7d window | 2.6 ms | 2.2 ms | slightly faster (chunk exclusion) |
| b1610, 1 station | 152 ms | 165 ms | unchanged — seq scan from non-sargable `settlement_date + interval` expr (pre-existing, orthogonal to TS) |
| **annual summary, per station** | **0.6 ms** | **55 ms** | **regression** — FULL OUTER JOIN view defeats predicate pushdown |

## Storage (with columnar compression on cold chunks)

| Table | Vanilla | Compressed | Ratio |
|---|--:|--:|--:|
| bmu_dispatch | 1551 MB | 37 MB | 42× |
| pn | 312 MB | 9 MB | 36× |
| mels | 366 MB | 15 MB | 24× |
| b1610 | 305 MB | 23 MB | 13× |
| constraints | 104 MB | 12 MB | 8× |
| boalf | 60 MB | 8 MB | 8× |
| **total** | **2.7 GB** | **114 MB** | **24×** |

NB an early throwaway-clone test suggested compressed reads were ~15× slower, but
the authoritative production benchmark (see table above) shows reads stayed fast —
even improved — on compressed chunks, because `segmentby` matches the dashboard
filter columns (`bmu`, `constraint_group`, `ngc_bm_unit`). The real reason to keep
the 30-day compression threshold is **DML, not reads**: the ingester upserts into
recent chunks, and upserting into compressed chunks is expensive — so hot chunks
must stay row-store.

## Findings & gotchas

1. **Latent tz bug found.** The existing rollup's `(ts AT TIME ZONE 'UTC')::date`
   re-localises to the `Europe/London` session TZ → days bucket on BST midnight,
   not UTC. The CAGG's `time_bucket(INTERVAL '1 day', ts)` on naive-UTC is correct.
   Grand totals match (1730124.4 MWh); only day boundaries differ. TS *fixes* this.
2. **Chunk sizing matters.** 1-day chunks on the 6.6-year `constraints` table made
   2349 chunks (bigger than vanilla, broke bulk compression). 30-day chunks → 82.
3. **CAGG view read regression** (annual summary): the `FULL OUTER JOIN` of the two
   CAGGs with `COALESCE` join/filter columns blocks pushdown → fixed ~50 ms scan
   regardless of window. Matters less at year-scale reads; fixable via LEFT JOIN
   with the filter on `d.bucket`, or a thin materialised table fed by the CAGGs.
4. **pandas_dispatch control** got ~3× faster (778→255 ms) — chunk exclusion speeds
   the per-unit source reads feeding the interpolation. Bonus, partly cache.

---

# Ingest upsert paths — COPY vs executemany

Methodology: `bench/upsert_paths.py`, dev Postgres over TCP (docker compose),
synthetic fuelinst-shaped rows, throwaway target table truncated between runs.
`copy` = COPY into an unlogged staging table + one `INSERT … SELECT … ON
CONFLICT`; `executemany` = the previous row-wise path (psycopg3 pipelines it).

| rows | plain table | plain, all conflict | **hypertable** | **hypertable, all conflict** |
|---|--:|--:|--:|--:|
| 500 | 0.7× | 0.6× | 2.2× | 1.7× |
| 5,000 | 1.3× | 1.0× | 2.3× | 2.7× |
| 50,000 | 1.2× | 1.2× | 2.7× | 3.5× |
| 250,000 | 1.2× | 1.2× | 3.1× | 3.4× |

(Speedup = executemany ÷ copy. 250k rows into a hypertable: 10.4s → 3.3s.)

**Takeaway.** On plain tables COPY is barely worth it (~1.2×) and is a net loss
for small batches. On hypertables — which is every real ingest target — it is
2–3.5×, and the gap widens with batch size. psycopg3's pipelined `executemany`
is much stronger than the usual "COPY is 10–100× faster" folklore suggests; the
hypertable win comes from chunk routing being paid once per statement rather
than once per row.

`COPY_MIN_ROWS = 500` keeps small batches (the hourly scheduler ticks) on the
executemany path, where the staging round-trip would otherwise dominate.

---

# Ingest fetch concurrency

Methodology: `bench/ingest_concurrency.py`, boalf over 3–9 June 2026 (24 windows
of 6h, 156,481 rows), live Elexon API, dev Postgres. First pass discarded so
every timed pass is an equal all-rows-conflict update.

| phase | concurrency 1 | 4 | 8 |
|---|--:|--:|--:|
| fetch only | 4.69s | **1.80s** | 1.81s |
| write only (always serial) | 6.09s | — | — |

| end to end | 1 | 2 | 4 | 8 |
|---|--:|--:|--:|--:|
| `run_windows` | 11.79s | 7.15s | **6.83s** | 6.94s |

**Takeaway.** Fetch parallelises ~2.6× and plateaus at 4 threads — more just
queues behind the API. Writes are serial by design (one connection, ordered for
last-write-wins), so **the serial write time is the hard floor**: 6.09s here,
against 6.83s achieved. End to end that is 1.7× for this dataset, and no
concurrency setting can do better without parallelising the writes too.

Getting there required pipelining rather than batching: the first implementation
fetched a batch of N, then wrote it with the pool idle, and landed at ~9.4s
(1.05× — almost nothing). Submitting the next fetch *before* writing the current
batch overlaps the two phases and recovers the rest. Datasets whose payloads
dominate (b1610) or any deployment with higher API latency should gain more;
`GB_GRID_FETCH_CONCURRENCY` tunes it.

---

# Rust ingester — golden equivalence + CPU

The ingest + materialize pipelines are ported to a standalone Rust binary
(`ingester/`, built as `gb-grid-ingest`). Equivalence is checked by running both
implementations over the **same real data** in two scratch databases and diffing
the resulting `bmu_dispatch` table (`bench/rs_vs_py/golden_materialize.sh`).
Timings via bash `time` (real/user/sys); Python materialize needs
`LD_LIBRARY_PATH` for the venv's numpy (NixOS), which the script prepends.

## Materialize (`materialize-dispatch`, 4 workers)

Real BMRS data sliced from the dev DB (PN/BOALF/MELS/B1610) into both scratch
DBs, then `materialize_dispatch` run by each implementation. 5-min grid, every
active BMU.

| Window | Rows | Python wall | Rust wall | Python user | Rust user |
|---|--:|--:|--:|--:|--:|
| 1 day (2026-05-01) | 702,146 | 37.1s | 13.0s (**2.9×**) | 81.5s | 4.5s (**18×**) |
| 3 days (05-01..05-03) | 2,109,602 | 96.8s | 33.8s (**2.9×**) | 194.0s | 12.9s (**15×**) |

**Output is byte-identical** between the two implementations on both windows
(2.1M rows diffed clean; only the *sign of zero* in `so_curtailment_mw`
differs — Python stores `-0.0` (numpy clip on a signed zero), Rust `+0.0`, which
are numerically identical and ignored by the comparison).

**Takeaway.** Rust cuts materialize CPU ~15–18× and wall-clock ~2.9×. The
Python CPU number exceeds wall-clock because the process pool burns 4 cores;
Rust's interp is a ~25-line sweep over sorted segments instead of numpy
masked-stamps fanned across a pool, so even at 4 workers it uses a fraction of
one core. The CPU floor (the DB write) is unchanged — both write the same rows
to the same Postgres.

## CLI startup (per-invocation fixed cost)

| Command | Python wall/user | Rust wall/user |
|---|--:|--:|
| `status` (imports pandas) | 0.60s / 0.42s | 0.17s / 0.004s |

Rust startup is ~4× faster wall and ~100× less CPU — the pandas/import cost is
gone entirely. For the always-on scheduler the per-tick saving is the
materialize win above; for one-off CLI invocations (backfill chunks, `status`)
the startup saving applies every time.

## How to reproduce

```bash
# in nix develop (dev Postgres up, GB_GRID_DATABASE_URL exported)
cargo build --release        # in ingester/
bash bench/rs_vs_py/golden_materialize.sh --from 2026-05-01 --to 2026-05-03 --workers 4
```

The script creates per-process scratch DBs (`gb_grid_golden_py_<pid>` /
`gb_grid_golden_rs_<pid>`), migrates both via the Python `gb-grid migrate`,
copies the real window slice, runs both `materialize-dispatch`, diffs the
`bmu_dispatch` output, and prints the timing table. It exits non-zero if the
tables differ.

## Full pipeline, winter + clock changes (Nov 2026)

`bench/rs_vs_py/golden_pipeline.sh` runs both implementations end to end
against the **live** Elexon API, each into its own scratch DB: `backfill` (all
datasets), then `materialize-dispatch`, then `materialize-dispatch-daily`. It
diffs every ingested table, the watermarks, `bmu_dispatch`,
`bmu_dispatch_daily` and every continuous aggregate.
`golden_constraints.sh` does the same for `refresh-constraints` over the full
NESO CSV (Jan 2023 onward, eight clock changes).

| Window | Local day lengths | `bmu_dispatch` rows | Ingest + dispatch | Aggregates |
|---|---|--:|---|---|
| 2026-01-13..14 (GMT) | 24, 24 h | 1,374,336 | identical | ≤1e-12 |
| 2025-10-25..27 (fall back) | 24, **25**, 24 h | 1,973,808 | identical | ≤1e-12 |
| 2026-03-28..30 (spring fwd) | 24, **23**, 24 h | 2,096,928 | identical | ≤1e-12 |
| 2026-10-24..26 (fall back) | 24, **25**, 24 h | 2,244,672 | identical | ≤1e-12 |
| constraints, full CSV | 8 transitions | 776,108 | identical | n/a |

Every settlement-date dataset has 46 periods on the spring day and 50 on the
fall-back days. Every BMU has ≤288 five-minute points per UTC day (no doubled
hour). **Aggregates differ only in float summation order.** A CAGG averages
rows in the table's physical order, and the two DBs store identical rows in
different orders (worker-completion order). Recomputing with `avg(x ORDER BY
ts)` gives identical values in both DBs. The largest relative difference seen
was 4.6e-16. `bmu_dispatch_daily` is built from a CAGG, so the script applies
the same 1e-12 tolerance to it. Base tables must match exactly.

Materialize: Rust 2.9–3.1× faster wall-clock, 23–30× less CPU (e.g. 3 days:
48.9s / 92.0s user → 16.0s / 3.1s). Backfill takes about the same wall-clock
in both (it waits on the API) and Rust uses about 30% less CPU.

### Bugs this found

In the Rust port (fixed, with regression tests):

- **JSON float rounding.** serde_json's default parser is not correctly
  rounded: `122.79999999999999` came out as `122.8`, 1 ULP from Python's value
  (`system_prices`). Fixed by enabling the `float_roundtrip` feature.
- **Constraints timestamps without seconds.** 84,575 rows of the live CSV
  (`2023-10-28T00:00`) were dropped, because Rust only accepted `%H:%M:%S`.
  It now accepts every form `fromisoformat` does.

Upstream API changes (fixed in both implementations; prod is broken on both
until it runs the fixed ingester):

- **Elexon B1610/stream** now returns 400 for `from`/`to`. It needs
  `settlementDateFrom`/`To` plus `settlementPeriodFrom=1`/`To=50`. Without the
  period bounds it returns period 1 only. 50 covers the long fall-back day.
- **NESO constraints CSV** renamed its columns to `Date_ Time GMT_BST`,
  `Limit_MW` and `Flow_MW`, with ISO `T` timestamps. Both parsers skipped every
  row and reported success with 0 rows. A missing column is now a hard error.

Pre-existing in the shared SQL/semantics, same in both (not fixed):

- B1610 sub-daily time axis (`b1610_series`, heatmap panels) uses
  `(date + (sp-1)*30min) AT TIME ZONE 'Europe/London'`. On transition days that
  misplaces 42 of 46 (spring) or 48 of 50 (autumn) periods by an hour, and
  collides two half-hours. The fix is to anchor at local midnight:
  `((date::timestamp AT TIME ZONE 'Europe/London') + (sp-1)*30min) AT TIME ZONE 'UTC'`.
- Constraints on fall-back days: the CSV lists 01:00 and 01:30 twice (BST,
  then GMT). Both parsers resolve each copy to the BST instant (fold=0), and
  last-write-wins keeps the GMT values there. Each group loses two half-hours,
  01:00–02:00 UTC stays empty, and two half-hours carry GMT-time values. The
  fix is to resolve the second copy of a repeated wall-clock time as fold=1.

```bash
bash bench/rs_vs_py/golden_pipeline.sh --from 2025-10-25 --to 2025-10-27   # [--keep]
bash bench/rs_vs_py/golden_constraints.sh                                  # [--keep]
```
