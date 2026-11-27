# gb-grid

**gb-grid** builds and continuously updates a queryable database of Great
Britain's electricity grid, sourced from
[Elexon's BMRS Insights API](https://data.elexon.co.uk/bmrs/api/v1) — the
official near-real-time data feed for the GB electricity balancing and
settlement system. It downloads generation, balancing-action, and
plant-capacity data, stores it in a [TimescaleDB](https://www.timescale.com/)
(PostgreSQL) database, and ships a set of [Grafana](https://grafana.com/)
dashboards on top for exploration.

It's aimed at anyone who wants to poke at how the GB grid actually behaves —
energy hobbyists, researchers, students, or developers prototyping against real
balancing-mechanism data. With it you can explore questions like:

- How much wind generation is being **curtailed** (paid to switch off), and at which wind farms?
- What is each power station actually generating, versus what it planned to?
- How is the system operator **dispatching** individual units minute to minute?
- How does the national fuel mix (wind / gas / nuclear / interconnectors / …) shift across a day?

The whole stack runs from a single `docker compose up` — Postgres, Grafana, and
a background ingester that keeps the data current. If you use [Nix](https://nixos.org/),
there's a dev shell layered on the same services.

![BMU dispatch dashboard](docs/images/bmu_dispatch.png)

> [!CAUTION]
> This repo is almost entirely vibe-coded. Do not expect quality, but also, let me assure you it would be even worse if I attempted to write this myself.

## Quick start

The whole stack runs through `docker compose` — Postgres (TimescaleDB), Grafana
(provisioned with the dashboards), and the ingester.

```bash
docker compose up -d --build          # or: just docker-up
docker compose run --rm app gb-grid backfill --from 2026-04-01 --to 2026-05-01
docker compose run --rm app gb-grid materialize-dispatch --from 2026-04-01 --to 2026-05-01
open http://localhost:3000            # Grafana — anonymous Editor; admin/admin to log in
```

`materialize-dispatch` builds the per-BMU dispatch series the dispatch
dashboards read from; run it after each backfill (the always-on ingester rolls
it forward automatically from then on).

Postgres is on host `127.0.0.1:5433` (db/user `gb_grid`, password `gbgrid` by
default — override via a `.env`, see `.env.example`). The `app` service applies
migrations then runs the always-on ingester; one-off CLI commands run as
`docker compose run --rm app gb-grid <cmd>`.

> The `app` image (and the `just` recipes) still wrap the original Python CLI,
> `gb-grid`, which is **deprecated**. It works for trying the stack out. The
> maintained ingester is the Rust binary `gb-grid-ingest` (see [CLI](#cli)), and
> that is what the NixOS deployment runs.

> The TimescaleDB and Grafana images are pulled from their official registries
> at run time — this repo only ships its own app image and config, so it
> doesn't redistribute either of them.

## Developing (Nix)

`nix develop` is a thin toolchain shell (cargo/rustc, Python + uv for migrations
and the golden tests, ruff, just, psql client)
layered on top of the **same** Docker services — it is not a second way to run
the stack. Entering it runs `docker compose up -d db grafana`, syncs `.venv`,
and applies migrations, with `GB_GRID_DATABASE_URL` and the `PG*` vars pointed at
the compose Postgres (`127.0.0.1:5433`, user/password `gb_grid`/`gbgrid`). So
`psql`, `pg_dump`, GUI clients, the test suite, and the CLI all run natively
against the containerized DB:

```bash
nix develop
cargo build --release --manifest-path ingester/Cargo.toml
alias gb-grid-ingest=$PWD/ingester/target/release/gb-grid-ingest
gb-grid-ingest backfill --from 2026-04-01 --to 2026-05-01
gb-grid-ingest materialize-dispatch --from 2026-04-01 --to 2026-05-01   # per-BMU dispatch series for the dashboards
gb-grid-ingest run         # always-on ingester (run under systemd in prod)
gb-grid-ingest status
psql                       # the PG* vars already point at the compose DB
```

`cargo test` in `ingester/` runs the Rust suite. It creates and migrates a
throwaway database on the compose Postgres. `nix run .#gb-grid-ingest -- <cmd>` runs the packaged binary instead
of a local build.

Requires the Docker daemon (`virtualisation.docker.enable` on NixOS). The
`app`/ingester container is left stopped in the devShell so you can drive it
natively; `just docker-up` runs the full always-on stack instead.

## What's in the database

Most data is keyed by **BMU** (Balancing Mechanism Unit) — the grid's unit of
account for a generator or large consumer (a wind farm, a gas turbine, an
interconnector, a battery). The `bmu` table maps each BMU to a power station,
fuel type, and map coordinates; the time-series tables below hang off it.

| Table | What it is | Cadence |
|---|---|---|
| `bmu` | Static registry: BMU → station, fuel type, latitude/longitude | one-off (vendored) |
| `fuelinst` | National generation broken down by fuel type (wind, gas, nuclear, interconnectors, …) | 5 min |
| `b1610` | Actual metered generation, per BMU (settlement-grade actuals) | 30 min, ~5 working days lag |
| `pn` | Physical Notifications — each unit's own planned output | 5 min |
| `boalf` | Balancing acceptances — the system operator's instructions telling units to change output | 5 min |
| `mels` | Maximum Export Limits — the most each unit can currently export (its live available capacity) | 5 min |
| `system_prices` | Imbalance settlement prices | hourly |
| `constraints` | NESO day-ahead network constraint flows | daily |

From these, `materialize-dispatch` derives `bmu_dispatch` — a per-BMU,
5-minute reconciliation of planned vs. instructed vs. actual output, including
**turn-up** (instructed to generate more) and **curtailment** (instructed to
generate less, e.g. wind paid off when the network is congested). That's what
the dispatch dashboards read.

## Data sources

- **[Elexon BMRS Insights API](https://bmrs.elexon.co.uk/)** — the live grid
  data feed (`fuelinst`, `b1610`, `pn`, `boalf`, `mels`, `system_prices`), plus
  its BMU reference list (units, names, fuel types). Free, no key required.

  > Elexon data is accessed under the [following open license terms](https://www.elexon.co.uk/bsc/data/balancing-mechanism-reporting-agent/copyright-licence-bmrs-data/)
  
- **[NESO](https://www.neso.energy/data-portal)** (National Energy System
  Operator) — the day-ahead constraint flows, and the BMU registry behind the
  original `bmu` table.
- **[OSUKED](https://osuked.github.io/Power-Station-Dictionary/)** — BMU → fuel
  type and power-station mapping.
- **[REPD](https://www.gov.uk/government/publications/renewable-energy-planning-database-quarterly-extract)**
  (the UK government's Renewable Energy Planning Database) — renewable and
  storage site coordinates.
- **[Wikidata](https://www.wikidata.org/)** — power stations tagged with their
  National Grid BM unit ID (P11610), which gives exact BMU → coordinates links.
- **[DUKES 5.11](https://www.gov.uk/government/statistics/electricity-chapter-5-digest-of-united-kingdom-energy-statistics-dukes)**
  (DESNZ's list of major power stations) — thermal and nuclear site coordinates.

The `bmu` table started as a one-off merge vendored as a CSV
(`src/gb_grid/migrations/0004_bmu.csv`). It is refreshed by
`tools/bmu/refresh_bmu.py` (migrations 0015/0016), which combines Elexon's BMU
list with Wikidata, REPD and DUKES. See
[docs/DATA_PIPELINE.md](docs/DATA_PIPELINE.md) for the matching rules and how
they're validated. Everything else is fetched live.

## Acknowledgements

The approach to ingesting and reconciling the BMRS streams draws heavily on
Energy Systems Catapult's
[`uk-live-generation`](https://github.com/ES-Catapult/uk-live-generation), which
served as a reference for a lot of the processing here.

## CLI

The ingester is the Rust binary `gb-grid-ingest` (`ingester/`, flake package
`.#gb-grid-ingest`):

- `gb-grid-ingest run` — always-on scheduler. It runs the ingest loops and rolls the materialized tables forward.
- `gb-grid-ingest backfill --from YYYY-MM-DD --to YYYY-MM-DD [-d fuelinst -d b1610 ...]` — historical load (all datasets by default)
- `gb-grid-ingest materialize-dispatch --from YYYY-MM-DD --to YYYY-MM-DD [--bmu PEHE-1 ...] [--workers N]` — recompute per-BMU dispatch series into `bmu_dispatch` (5-min resolution)
- `gb-grid-ingest materialize-dispatch-daily --from … --to …` — roll up to per-BMU daily MWh (`bmu_dispatch_daily`)
- `gb-grid-ingest refresh-caggs --from … --to …` — refresh every continuous aggregate over a window
- `gb-grid-ingest refresh-constraints` — download and upsert the NESO constraints CSV
- `gb-grid-ingest status` — row counts and latest timestamps

Schema migrations are still Python: `gb-grid migrate` applies pending yoyo
migrations, and migration 0004 is a Python step. The rest of the Python CLI
(`gb-grid run`, `backfill`, `materialize-*`, `sql`, …) is **deprecated**. It is
kept only as the reference implementation for the golden-equivalence scripts in
`bench/rs_vs_py/`.

## Dashboards

`docker compose` runs Grafana on <http://localhost:3000> (anonymous Editor
access; admin/admin if you want to log in), provisioned from
`docker/grafana/provisioning/` (datasource uid `gbgrid`, pointed at the `db`
service) plus the dashboards in `grafana/dashboards/`.

### BMU dispatch

Pick a station (or click one on the map) to see its aggregate planned vs.
instructed vs. actual output, a stacked per-BMU breakdown, and the turn-up /
curtailment the system operator instructed.

![BMU dispatch dashboard](docs/images/bmu_dispatch.png)

The bottom panel separates instructed **turn-up** (green, generate more) from
**curtailment** (red, generate less) per BMU:

![BMU dispatch — turn-up and curtailment](docs/images/bmu_dispatch_curtailment.png)

> [!NOTE]
> The PN / Dispatched / B1610 series on the station-total panel still need
> work: the three are sourced from different streams (interpolated PN+BOALF
> vs. half-hourly settlement actuals) and don't always reconcile cleanly,
> especially around BOA acceptances and during B1610's ~5-working-day lag.
> Treat them as a sanity check on each other, not as the same number.

### BMU output heatmap

National generation by fuel type, with per-station load-factor heatmaps (output
as a % of each station's MEL capacity) grouped by fuel — a quick read on which
plants are running hard and which are idle.

![BMU output heatmap dashboard](docs/images/bmu_heatmap.png)

### BMU annual summary

A year of daily output per station as a calendar heatmap, either in absolute
MWh/day or — pictured — as a % of MEL capacity, alongside a map for selecting
stations and fuel types.

![BMU annual summary (% of MEL capacity) dashboard](docs/images/bmu_annual_summary_pct.png)

Backfill, then materialize, then open Grafana:

```bash
gb-grid-ingest backfill --from 2026-04-01 --to 2026-04-07
gb-grid-ingest materialize-dispatch --from 2026-04-01 --to 2026-04-07
xdg-open http://localhost:3000/d/bmu-dispatch
```

## Migrations

Schema lives in `src/gb_grid/migrations/` as numbered SQL files, applied with
[yoyo-migrations](https://ollycope.com/software/yoyo/latest/). Add a new file
(e.g. `0017.add-foo.sql`) and `gb-grid migrate` will apply it on next run.
In production the NixOS module runs `ALTER EXTENSION timescaledb UPDATE`, then
`gb-grid migrate`, before the ingester starts. Never edit a migration that has
been merged. To correct one, add a new migration that supersedes it (as 0016
does for 0015).

## Configuration

| Env var | Purpose | Default |
|---|---|---|
| `GB_GRID_DATABASE_URL` | Postgres connection URL | _(set by devShell; required otherwise)_ |
| `PGHOST` / `PGPORT` / `PGUSER` / `PGPASSWORD` | libpq-style fallbacks for whatever the URL leaves out. For example, `postgresql:///gb_grid` connects over the local socket as the OS user | socket dir (`/run/postgresql`, …) / `$USER` |
| `GB_GRID_API_BASE` | BMRS API base URL | `https://data.elexon.co.uk/bmrs/api/v1` |
| `GB_GRID_HTTP_TIMEOUT` | HTTP timeout (seconds) | `60` |
| `GB_GRID_FETCH_CONCURRENCY` | Windows fetched in parallel per ingest | `4` |
| `GB_GRID_POLL_SECONDS` | Scheduler tick interval for every loop | `3600` |
| `GB_GRID_POLL_<DATASET>_SECONDS` | Override one loop; `<DATASET>` is `FUELINST`, `BOALF`, `B1610`, `PN`, `MELS`, `SYSTEM_PRICES`, `MATERIALIZE`, `MATERIALIZE_DAILY` | `GB_GRID_POLL_SECONDS` |

## Deployment

`nixosModules.default` runs Postgres + TimescaleDB, Grafana and the ingester.
- **Ingester:** pass the Rust ingester as `specialArgs.gb-grid-ingest-pkg` (from
  `packages.gb-grid-ingest`) and the module runs it for the scheduler and the
  constraints timer. Without it, the module falls back to the deprecated Python
  ingester. `gb-grid-pkg` (`packages.default`, the Python app) is still
  required for `gb-grid migrate`.
- **Start-up:** the service runs `ALTER EXTENSION timescaledb UPDATE`, then
  `gb-grid migrate`, then the ingester.
- **TimescaleDB version:** the module builds TimescaleDB ≥ 2.32.1, because
  nixos-26.05's 2.27.1 makes every TimescaleDB background job fail on
  PostgreSQL ≥ 16.16. The override drops itself once nixpkgs catches up.
- **No rollback past a TimescaleDB upgrade:** extension upgrades can't be
  undone. Don't roll back to a NixOS generation with an older TimescaleDB once
  a newer one has run.

The Rust ingester produces byte-identical output to the Python one, including
across winter and clock-change windows. The golden scripts are in
`bench/rs_vs_py/`. It's about 3× faster wall-clock and 15–30× cheaper in CPU on
materialize (`bench/RESULTS.md`).

I've deployed this myself in "production" at home, as a NixOS container on a
bare-metal NixOS host. See [`nix-config`](https://github.com/chrsphr/nix-config)
(`hosts/containers/gb-grid.nix`, wired up in `modules/container-host.nix`) for
the host recipe.
