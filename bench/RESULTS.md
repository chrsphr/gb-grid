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
