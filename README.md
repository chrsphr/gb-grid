# gb-grid

Local DuckDB database of GB power grid data, fed from the [Elexon BMRS Insights API](https://data.elexon.co.uk/bmrs/api/v1).

## Quick start

```bash
nix develop                # (or: nix-shell -p ... if you don't have flakes)
uv sync --extra dev
gb-grid backfill --from 2026-04-01 --to 2026-05-01
gb-grid run                # always-on ingester (run under systemd in prod)
gb-grid status
duckdb data/gb_grid.duckdb # interactive SQL
```

## Datasets

| Table | Source | Cadence |
|---|---|---|
| `fuelinst` | BMRS `FUELINST` | 5 min |
| `b1610` | BMRS `B1610` (per-BMU actuals) | 30 min, ~5 working days lag |
| `boalf` | BMRS `BOALF` (balancing acceptances) | 5 min |
| `system_prices` | BMRS settlement system prices | hourly |

## CLI

- `gb-grid backfill --from YYYY-MM-DD --to YYYY-MM-DD [--dataset fuelinst,b1610,...]`
- `gb-grid run` — always-on async scheduler
- `gb-grid status` — row counts and latest timestamps
