set shell := ["bash", "-cu"]

default:
    @just --list

sync:
    uv sync --extra dev

backfill FROM TO *DATASETS:
    gb-grid backfill --from {{FROM}} --to {{TO}} {{DATASETS}}

run:
    gb-grid run

status:
    gb-grid status

sql:
    gb-grid sql

test:
    pytest

lint:
    ruff check src tests

kernel:
    ./scripts/register-kernel.sh
