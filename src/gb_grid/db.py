from __future__ import annotations

from collections.abc import Iterable, Sequence
from datetime import datetime
from pathlib import Path
from typing import Any

import duckdb

from .config import DB_PATH

SCHEMA_DIR = Path(__file__).parent / "schema"


def connect(db_path: Path | str | None = None) -> duckdb.DuckDBPyConnection:
    path = Path(db_path) if db_path is not None else DB_PATH
    if str(path) != ":memory:":
        path.parent.mkdir(parents=True, exist_ok=True)
    conn = duckdb.connect(str(path))
    migrate(conn)
    return conn


def migrate(conn: duckdb.DuckDBPyConnection) -> None:
    conn.execute(
        "CREATE TABLE IF NOT EXISTS schema_migrations ("
        "  version TEXT PRIMARY KEY,"
        "  applied_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP"
        ")"
    )
    applied = {row[0] for row in conn.execute("SELECT version FROM schema_migrations").fetchall()}
    for sql_file in sorted(SCHEMA_DIR.glob("*.sql")):
        version = sql_file.stem
        if version in applied:
            continue
        conn.execute(sql_file.read_text())
        conn.execute("INSERT INTO schema_migrations(version) VALUES (?)", [version])


def upsert(
    conn: duckdb.DuckDBPyConnection,
    table: str,
    rows: Sequence[dict[str, Any]],
    conflict_cols: Sequence[str],
) -> int:
    """Upsert rows into ``table`` using DuckDB's bulk DataFrame ingestion.

    Builds a pandas DataFrame, registers it as a view, and runs a single
    ``INSERT … SELECT … ON CONFLICT DO UPDATE``. This is dramatically faster
    than ``executemany`` for non-trivial batches (typically 10–100×).
    """
    if not rows:
        return 0
    import pandas as pd  # local import keeps import cost off the hot path until needed

    cols = list(rows[0].keys())
    df = pd.DataFrame(rows, columns=cols)

    update_cols = [c for c in cols if c not in conflict_cols]
    set_clause = ", ".join(f"{c} = EXCLUDED.{c}" for c in update_cols)
    conflict = ", ".join(conflict_cols)
    select_list = ", ".join(cols)

    if set_clause:
        sql = (
            f"INSERT INTO {table} ({select_list}) "
            f"SELECT {select_list} FROM _gb_staging "
            f"ON CONFLICT ({conflict}) DO UPDATE SET {set_clause}"
        )
    else:
        sql = (
            f"INSERT INTO {table} ({select_list}) "
            f"SELECT {select_list} FROM _gb_staging "
            f"ON CONFLICT ({conflict}) DO NOTHING"
        )

    conn.register("_gb_staging", df)
    try:
        conn.execute("BEGIN")
        try:
            conn.execute(sql)
            conn.execute("COMMIT")
        except Exception:
            conn.execute("ROLLBACK")
            raise
    finally:
        conn.unregister("_gb_staging")
    return len(rows)


def set_watermark(conn: duckdb.DuckDBPyConnection, dataset: str, last_ts: datetime) -> None:
    conn.execute(
        "INSERT INTO ingest_watermark(dataset, last_ts, updated_at) "
        "VALUES (?, ?, CURRENT_TIMESTAMP) "
        "ON CONFLICT (dataset) DO UPDATE SET "
        "  last_ts = EXCLUDED.last_ts, updated_at = EXCLUDED.updated_at",
        [dataset, last_ts],
    )


def get_watermark(conn: duckdb.DuckDBPyConnection, dataset: str) -> datetime | None:
    row = conn.execute(
        "SELECT last_ts FROM ingest_watermark WHERE dataset = ?", [dataset]
    ).fetchone()
    return row[0] if row else None


def table_stats(conn: duckdb.DuckDBPyConnection, table: str, ts_col: str) -> dict[str, Any]:
    row = conn.execute(f"SELECT count(*), max({ts_col}) FROM {table}").fetchone()
    return {"table": table, "rows": row[0], "latest": row[1]}


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
