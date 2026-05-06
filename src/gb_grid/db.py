from __future__ import annotations

from collections.abc import Iterable, Sequence
from datetime import datetime
from pathlib import Path
from typing import Any

import psycopg
from psycopg.rows import dict_row
from yoyo import get_backend, read_migrations

from .config import database_url

MIGRATIONS_DIR = Path(__file__).parent / "migrations"


def _resolve_url(url: str | None = None) -> str:
    resolved = url or database_url()
    if not resolved:
        raise RuntimeError(
            "GB_GRID_DATABASE_URL is not set. "
            "In `nix develop` it's set automatically; otherwise export it."
        )
    return resolved


def connect(url: str | None = None) -> psycopg.Connection:
    return psycopg.connect(_resolve_url(url), autocommit=True)


def migrate(url: str | None = None) -> int:
    """Apply all pending yoyo migrations. Returns the number applied."""
    backend = get_backend(_resolve_url(url))
    migrations = read_migrations(str(MIGRATIONS_DIR))
    to_apply = backend.to_apply(migrations)
    n = len(list(to_apply))
    with backend.lock():
        backend.apply_migrations(backend.to_apply(migrations))
    return n


def upsert(
    conn: psycopg.Connection,
    table: str,
    rows: Sequence[dict[str, Any]],
    conflict_cols: Sequence[str],
) -> int:
    """Bulk upsert via a single ``INSERT … VALUES … ON CONFLICT DO UPDATE``."""
    if not rows:
        return 0

    cols = list(rows[0].keys())
    update_cols = [c for c in cols if c not in conflict_cols]
    col_list = ", ".join(cols)
    placeholders = "(" + ", ".join(["%s"] * len(cols)) + ")"
    conflict = ", ".join(conflict_cols)

    if update_cols:
        set_clause = ", ".join(f"{c} = EXCLUDED.{c}" for c in update_cols)
        tail = f"ON CONFLICT ({conflict}) DO UPDATE SET {set_clause}"
    else:
        tail = f"ON CONFLICT ({conflict}) DO NOTHING"

    values = [tuple(r.get(c) for c in cols) for r in rows]
    sql = f"INSERT INTO {table} ({col_list}) VALUES {placeholders} {tail}"

    with conn.cursor() as cur:
        cur.executemany(sql, values)
    return len(rows)


def set_watermark(conn: psycopg.Connection, dataset: str, last_ts: datetime) -> None:
    with conn.cursor() as cur:
        cur.execute(
            "INSERT INTO ingest_watermark(dataset, last_ts, updated_at) "
            "VALUES (%s, %s, CURRENT_TIMESTAMP) "
            "ON CONFLICT (dataset) DO UPDATE SET "
            "  last_ts = EXCLUDED.last_ts, updated_at = EXCLUDED.updated_at",
            (dataset, last_ts),
        )


def get_watermark(conn: psycopg.Connection, dataset: str) -> datetime | None:
    with conn.cursor() as cur:
        cur.execute(
            "SELECT last_ts FROM ingest_watermark WHERE dataset = %s", (dataset,)
        )
        row = cur.fetchone()
    return row[0] if row else None


def table_stats(conn: psycopg.Connection, table: str, ts_col: str) -> dict[str, Any]:
    with conn.cursor(row_factory=dict_row) as cur:
        cur.execute(f"SELECT count(*) AS rows, max({ts_col}) AS latest FROM {table}")
        row = cur.fetchone()
    return {"table": table, "rows": row["rows"], "latest": row["latest"]}


def iter_chunks(
    rows: Iterable[dict[str, Any]], size: int = 1000
) -> Iterable[list[dict[str, Any]]]:
    chunk: list[dict[str, Any]] = []
    for r in rows:
        chunk.append(r)
        if len(chunk) >= size:
            yield chunk
            chunk = []
    if chunk:
        yield chunk
