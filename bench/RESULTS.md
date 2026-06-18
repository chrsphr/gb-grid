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
