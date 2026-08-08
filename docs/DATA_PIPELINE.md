# gb-grid data pipeline

How raw BMRS/NESO data flows through ingest → derived series → multi-resolution
serving, and what each table/view/function produces. Reflects migrations 0001–0011.

The database is **PostgreSQL 16 + TimescaleDB**. The time-series tables are
hypertables; rollups are continuous aggregates; old chunks are compressed.

```
Elexon BMRS API ─┐
NESO CSV ────────┤ ingest (scheduler ticks / backfill CLI)
                 ▼
        raw hypertables: pn, boalf, mels, b1610, fuelinst, system_prices, constraints
                 │
                 │  materialize_dispatch  (pandas interpolation of segments → 5-min grid)
                 ▼
        bmu_dispatch  (fine 5-min, per-BMU)            ── hypertable, compressed
                 │
       ┌─────────┼───────────────────────────┐
       │ continuous aggregates (auto-refresh) │
       ▼                                       ▼
  bmu_dispatch_hourly / _daily_mw        bmu_dispatch_daily_cagg (sums)
  (avg MW, for dispatch_series)          b1610_daily_cagg (daily MWh)
                                                 │ materialize_dispatch_daily
                                                 ▼
                                          bmu_dispatch_daily (table, MWh — heatmap)
                 │
                 ▼  serving functions
        dispatch_series()  b1610_series()   → Grafana (resolution chosen by $__interval)
```

## 1. Ingest layer (raw hypertables)

Each dataset is pulled from the Elexon BMRS Insights API (constraints from a NESO
CSV) and upserted into a raw table. All are TimescaleDB hypertables (migration 0009)
partitioned on their time column; chunks older than 30 days (constraints: 60) are
compressed (columnar, `segmentby` = entity, `orderby` = time).

| Table | Grain | Key columns | Meaning |
|---|---|---|---|
| `pn` | segment | `national_grid_bm_unit, time_from→time_to, level_from→level_to` | **Physical Notification** — the unit's *planned* MW profile (piecewise-linear ramp). |
| `boalf` | segment | `acceptance_id, time_from→time_to, level_from→level_to, acceptance_time, so_flag` | **Bid-Offer Acceptance** — dispatch instructions that override PN. Piecewise-linear; later `acceptance_time` supersedes on overlap. `so_flag` = System Operator action. |
| `mels` | segment | `national_grid_bm_unit, time_from→time_to, level_from→level_to, notification_sequence` | **Maximum Export Limit** — physical cap. Higher `notification_sequence` supersedes on overlap. |
| `b1610` | half-hourly | `settlement_date, settlement_period, ngc_bm_unit, quantity_mwh` | **Actual metered generation** per BMU per settlement period (MWh). ~5 working-day publication lag. |
| `fuelinst` | instant | `publish_time, fuel_type, generation_mw` | Generation by fuel type. |
| `system_prices` | half-hourly | `settlement_date, settlement_period, ssp/sbp/niv` | Imbalance prices/volume. |
| `constraints` | time series | `constraint_group, ts, limit_mw, flow_mw` | NESO day-ahead constraint flows & limits (spans years; 30-day chunks). |
| `bmu` | static | `ngc_bm_unit, bm_unit, station, bmrs_fuel_type, lat/lon` | BMU registry metadata (not a hypertable). |
| `ingest_watermark` | — | `dataset, last_ts` | Last ingested timestamp per dataset. |

**How ingest runs** — the always-on scheduler (`gb-grid run`) loops per dataset
(`PollConfig`); windows are deliberately rolling to catch revisions:

| Dataset | Interval | Window |
|---|---|---|
| fuelinst | hourly | watermark (or now−2h) → now |
| boalf | hourly | now−4h → now |
| pn / mels | hourly | now−4h → now+24h |
| b1610 | hourly | today−14 → today−3 (inside the published zone) |
| system_prices | hourly | yesterday → today |
| constraints | daily timer | full NESO CSV (08:00 UTC, Mon–Fri) |

Every interval defaults to hourly and is configurable: `GB_GRID_POLL_SECONDS`
moves all of them, `GB_GRID_POLL_<DATASET>_SECONDS` (e.g.
`GB_GRID_POLL_FUELINST_SECONDS=300`) overrides one. Each window is wider than an
hourly tick, so the rolling overlap holds at the default.

Historical loads use `gb-grid backfill --from --to [--dataset …]`.

**Ingest performance.** Windows within one dataset are fetched
`GB_GRID_FETCH_CONCURRENCY` at a time (default 4) so HTTP latency overlaps
rather than accumulating; writes stay serial and in window order, preserving
last-write-wins and watermark semantics. Fetches are kept in flight *during* the
writes, so a backfill runs at roughly its serial write time (~1.7× faster end to
end; the write phase is the floor — see `bench/RESULTS.md`). Raising the setting
past 4 does nothing: the API stops parallelising there. Batches of ≥500 rows are written by
`COPY` into a staging table plus a single merge, which is 2–3.5× faster than
row-wise upsert on hypertables (see `bench/RESULTS.md`); smaller batches take the
row-wise path. Only 429/5xx responses are retried — other 4xx fail immediately
instead of burning five backed-off attempts on a request that cannot succeed.

The b1610 tick refreshes its continuous aggregates only for windows older than
the policies' 30-day `start_offset`; anything newer is already refreshed
automatically every 30 minutes, so backfills still repair history while the
hourly tick does no redundant work.

`constraints` sends `If-None-Match`/`If-Modified-Since` (validators cached in
`ingest_http_cache`) and skips the download entirely on a 304.

## 2. Derived: per-BMU dispatch (`bmu_dispatch`)

`materialize_dispatch` (scheduler tick: hourly over now−6h → now; or via
`gb-grid materialize-dispatch`) computes the **dispatch series** by interpolating
the PN/BOA/MEL segments onto a fixed **5-minute grid** per BMU (pandas, in
`analytics.bmu_dispatch`, fanned across a process pool). Written to the
`bmu_dispatch` hypertable (1-day chunks, compressed).

Per (`bmu`, `ts`):
- `pn_mw` — PN level, clipped to MEL.
- `boa_level_mw` — dispatched level: the winning BOA acceptance (later `acceptance_time` wins), clipped to MEL; falls back to `pn_mw` where no acceptance is active.
- `mel_mw` — maximum export limit.
- `so_turnup_mw` = max(boa_level − pn, 0) — SO instructed *up*.
- `boa_curtailment_mw` = max(pn − boa_level, 0) — instructed *down*.
- `so_curtailment_mw` — as above but only `so_flag` acceptances.

This is the only genuinely procedural step (linear-ramp resampling with
overlap-precedence + MEL capping); it can't be expressed as a SQL aggregate.

## 3. Continuous aggregates (auto-refreshing rollups)

All are TimescaleDB continuous aggregates with refresh policies (start_offset
30–90 days so revisions keep folding in).

| CAGG | Migration | Grain | Content | Consumed by |
|---|---|---|---|---|
| `bmu_dispatch_daily_cagg` | 0009 | day × bmu | **sums** of each MW metric | `bmu_dispatch_daily` table |
| `b1610_daily_cagg` | 0009 | day × bmu | `SUM(quantity_mwh)` | daily table + `b1610_series` daily tier |
| `bmu_dispatch_hourly` | 0010 | hour × bmu | **avg** of each MW metric | `dispatch_series` hourly tier |
| `bmu_dispatch_daily_mw` | 0010 | day × bmu | **avg** of each MW metric | `dispatch_series` daily tier |

`time_bucket` buckets on the naive-UTC `ts`, i.e. **true UTC days** (this fixed a
prior session-timezone bug where days bucketed on BST).

## 4. Derived: daily energy table (`bmu_dispatch_daily`)

`materialize_dispatch_daily` (scheduler tick: every 30 min over today−14 → today)
refreshes this **plain indexed table** by reading the two daily CAGGs (cheap —
joining ~tens of thousands of pre-aggregated rows, not the 100M-row fine table)
and scaling MW→MWh. Per (`bmu`, `date`): `pn_mwh, boa_dispatched_mwh,
so_turnup_mwh, boa_curtailment_mwh, so_curtailment_mwh, b1610_mwh`. This is the
source for the annual heatmap.

## 5. Serving functions (resolution matched to zoom)

Grafana passes the pixel-derived bucket width via `$__interval`; these routers
pick the coarsest tier whose native bucket ≤ the requested bucket, so every zoom
returns ~1–2k points in low-ms.

- **`dispatch_series(station, from, to, bucket)`** (0010) — per-BMU avg MW per
  bucket. `bucket ≥ 1 day` → `bmu_dispatch_daily_mw`; `≥ 1 hour` → `bmu_dispatch_hourly`;
  else raw `bmu_dispatch` (5-min). Panels sum across BMUs or break out per BMU.
- **`b1610_series(station, from, to, bucket)`** (0011) — station actual MW per
  bucket. `bucket ≥ 1 day` → `b1610_daily_cagg` (energy/24h); else raw half-hourly
  `b1610` with a `settlement_date` filter for chunk exclusion.

## 6. Dashboards (what reads what)

| Dashboard | Panels | Source |
|---|---|---|
| `bmu_dispatch` | PN/Dispatched/MEL + B1610; per-BMU; curtailment | `dispatch_series()` + `b1610_series()` |
| `bmu_annual_summary` | weekly/daily heatmaps | `bmu_dispatch_daily` |
| `constraints` | flow / limit / bands | `constraints` table |

## 7. Compression & retention

Migration 0009 enables columnar compression on the large hypertables with a policy
that compresses chunks older than 30 days (constraints 60). Hot/recent chunks stay
row-store so the ingester's upserts (and revisions) remain cheap; cold chunks
compress ~20–40×. `segmentby` is the entity column (e.g. `bmu`) so per-entity
dashboard reads stay fast even against compressed chunks.
