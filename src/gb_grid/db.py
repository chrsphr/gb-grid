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

# Batch size at which COPY-into-staging overtakes row-wise executemany.
COPY_MIN_ROWS = 500


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


def refresh_continuous_aggregate(
    conn: psycopg.Connection, cagg: str, start: Any, end: Any
) -> None:
    """Refresh continuous aggregate ``cagg`` over ``[start, end)``.

    ``refresh_continuous_aggregate()`` cannot run inside a transaction block, so
    ``conn`` must be autocommit (as :func:`connect` returns). The cagg name is an
    internal constant (never user input) and is interpolated directly: the
    procedure's first argument is a ``regclass`` and so cannot be a bind
    parameter.
    """
    with conn.cursor() as cur:
        cur.execute(
            f"CALL refresh_continuous_aggregate('{cagg}', %s, %s)", (start, end)
        )


def refresh_caggs(
    conn: psycopg.Connection, caggs: Iterable[str], start: Any, end: Any
) -> None:
    """Refresh several continuous aggregates over the same window."""
    for cagg in caggs:
        refresh_continuous_aggregate(conn, cagg, start, end)


def upsert(
    conn: psycopg.Connection,
    table: str,
    rows: Sequence[dict[str, Any]],
    conflict_cols: Sequence[str],
) -> int:
    """Bulk upsert. Small batches go via ``executemany``; large ones via ``COPY``.

    ``COPY`` into an unlogged staging table plus a single ``INSERT … SELECT``
    merge beats row-wise ``executemany`` by a wide margin once batches get big
    (backfills, the ~1M-row constraints CSV), because the rows cross the wire as
    one stream and the merge is one statement. Below the threshold the staging
    round-trip costs more than it saves.
    """
    if not rows:
        return 0
    if len(rows) < COPY_MIN_ROWS:
        return _upsert_values(conn, table, rows, conflict_cols)
    return _upsert_copy(conn, table, rows, conflict_cols)


def _conflict_tail(cols: Sequence[str], conflict_cols: Sequence[str]) -> str:
    conflict = ", ".join(conflict_cols)
    update_cols = [c for c in cols if c not in conflict_cols]
    if not update_cols:
        return f"ON CONFLICT ({conflict}) DO NOTHING"
    set_clause = ", ".join(f"{c} = EXCLUDED.{c}" for c in update_cols)
    return f"ON CONFLICT ({conflict}) DO UPDATE SET {set_clause}"


def _upsert_values(
    conn: psycopg.Connection,
    table: str,
    rows: Sequence[dict[str, Any]],
    conflict_cols: Sequence[str],
) -> int:
    cols = list(rows[0].keys())
    col_list = ", ".join(cols)
    placeholders = "(" + ", ".join(["%s"] * len(cols)) + ")"
    sql = (
        f"INSERT INTO {table} ({col_list}) VALUES {placeholders} "
        f"{_conflict_tail(cols, conflict_cols)}"
    )
    values = (tuple(r.get(c) for c in cols) for r in rows)

    with conn.cursor() as cur:
        cur.executemany(sql, list(values))
    return len(rows)


def _upsert_copy(
    conn: psycopg.Connection,
    table: str,
    rows: Sequence[dict[str, Any]],
    conflict_cols: Sequence[str],
) -> int:
    cols = list(rows[0].keys())
    col_list = ", ".join(cols)
    conflict = ", ".join(conflict_cols)
    stage = f"_stage_{table}"

    # Deduplicate inside the batch before merging. `executemany` applies rows one
    # statement at a time, so repeated keys within a batch just overwrite in
    # order; a single INSERT … SELECT would instead abort with "ON CONFLICT DO
    # UPDATE command cannot affect row a second time". MELS and BOALF both carry
    # revisions that hit this. `_ord` (filled by COPY from its DEFAULT) preserves
    # arrival order so DISTINCT ON keeps the last row per key, matching the
    # last-write-wins behaviour of the executemany path.
    with conn.transaction(), conn.cursor() as cur:
        cur.execute(
            f"CREATE TEMP TABLE {stage} (LIKE {table}) ON COMMIT DROP; "
            f"ALTER TABLE {stage} ADD COLUMN _ord bigserial"
        )
        with cur.copy(f"COPY {stage} ({col_list}) FROM STDIN") as copy:
            for r in rows:
                copy.write_row(tuple(r.get(c) for c in cols))
        cur.execute(
            f"INSERT INTO {table} ({col_list}) "
            f"SELECT DISTINCT ON ({conflict}) {col_list} FROM {stage} "
            f"ORDER BY {conflict}, _ord DESC "
            f"{_conflict_tail(cols, conflict_cols)}"
        )
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
