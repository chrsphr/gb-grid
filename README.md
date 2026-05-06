# gb-grid

> [!CAUTION]
> This repo is almost entirely vibe-coded. Do not expect quality, but also, let me assure you it would be even worse if I attempted to write this myself.

Postgres database of GB power grid data, fed from the [Elexon BMRS Insights API](https://data.elexon.co.uk/bmrs/api/v1).

## Quick start

```bash
nix develop                # auto-starts an ephemeral Postgres in .postgres/
uv sync --extra dev
gb-grid migrate            # apply yoyo migrations (the devShell does this for you)
gb-grid backfill --from 2026-04-01 --to 2026-05-01
gb-grid run                # always-on ingester (run under systemd in prod)
gb-grid status
gb-grid sql                # opens psql against $GB_GRID_DATABASE_URL
```

The devShell exports `GB_GRID_DATABASE_URL` and standard `PG*` vars, so `psql`,
`pg_dump`, GUI clients (host `127.0.0.1`, port `5433`, user = `$USER`, no
password), the test suite, and the CLI all just work.

## Datasets

| Table | Source | Cadence |
|---|---|---|
| `fuelinst` | BMRS `FUELINST` | 5 min |
| `b1610` | BMRS `B1610` (per-BMU actuals) | 30 min, ~5 working days lag |
| `boalf` | BMRS `BOALF` (balancing acceptances) | 5 min |
| `pn` | BMRS `PN` (physical notifications) | 5 min |
| `mels` | BMRS `MELS` (max export levels) | 5 min |
| `system_prices` | BMRS settlement system prices | hourly |

## CLI

- `gb-grid migrate` — apply pending yoyo migrations
- `gb-grid backfill --from YYYY-MM-DD --to YYYY-MM-DD [--dataset fuelinst,b1610,...]`
- `gb-grid run` — always-on async scheduler
- `gb-grid status` — row counts and latest timestamps
- `gb-grid sql` — open `psql` against the configured database

## Migrations

Schema lives in `src/gb_grid/migrations/` as numbered SQL files, applied with
[yoyo-migrations](https://ollycope.com/software/yoyo/latest/). Add a new file
(e.g. `0002.add-foo.sql`) and `gb-grid migrate` will apply it on next run.

## Configuration

| Env var | Purpose | Default |
|---|---|---|
| `GB_GRID_DATABASE_URL` | Postgres connection URL | _(set by devShell; required otherwise)_ |
| `GB_GRID_API_BASE` | BMRS API base URL | `https://data.elexon.co.uk/bmrs/api/v1` |
| `GB_GRID_HTTP_TIMEOUT` | HTTP timeout (seconds) | `60` |

## Deployment

Deployed as a NixOS LXC container — see
[`nix-config/gb-grid.nix`](https://github.com/chrsphr/nix-config) for the host
recipe. The flake exposes `packages.default` (the Python app) for downstream
consumers.
