"""Test fixtures.

The flake's devShell starts an ephemeral Postgres in ``$PWD/.postgres/`` and
exports ``GB_GRID_DATABASE_URL``. Tests piggyback on that cluster: a session-
scoped fixture creates a sibling ``gb_grid_test`` database, runs migrations
against it, and drops it at the end. Each test gets a clean connection with
all tables truncated.
"""

from __future__ import annotations

import os
from urllib.parse import urlparse, urlunparse

import psycopg
import pytest

from gb_grid.db import migrate

TEST_DB = "gb_grid_test"

_TABLES = ["fuelinst", "b1610", "boalf", "pn", "mels", "system_prices", "ingest_watermark"]


def _swap_dbname(url: str, dbname: str) -> str:
    """Replace the database name in a postgres URL, preserving the host=... query."""
    parts = urlparse(url)
    new = parts._replace(path=f"/{dbname}")
    rebuilt = urlunparse(new)
    # urlunparse drops the empty authority, producing "postgresql:/db?..." rather
    # than "postgresql:///db?...". Re-insert the missing slashes.
    if rebuilt.startswith("postgresql:/") and not rebuilt.startswith("postgresql:///"):
        rebuilt = rebuilt.replace("postgresql:/", "postgresql:///", 1)
    return rebuilt


def _admin_url() -> str:
    url = os.environ.get("GB_GRID_DATABASE_URL")
    if not url:
        raise RuntimeError(
            "GB_GRID_DATABASE_URL not set — run tests inside `nix develop`."
        )
    return _swap_dbname(url, "postgres")


@pytest.fixture(scope="session", autouse=True)
def _test_database():
    admin = _admin_url()
    base = os.environ["GB_GRID_DATABASE_URL"]
    test_url = _swap_dbname(base, TEST_DB)

    with psycopg.connect(admin, autocommit=True) as conn, conn.cursor() as cur:
        cur.execute(f"DROP DATABASE IF EXISTS {TEST_DB} WITH (FORCE)")
        cur.execute(f"CREATE DATABASE {TEST_DB}")

    os.environ["GB_GRID_DATABASE_URL"] = test_url
    migrate()
    try:
        yield test_url
    finally:
        os.environ["GB_GRID_DATABASE_URL"] = base
        with psycopg.connect(admin, autocommit=True) as conn, conn.cursor() as cur:
            cur.execute(f"DROP DATABASE IF EXISTS {TEST_DB} WITH (FORCE)")


@pytest.fixture
def db():
    """Fresh connection with all tables truncated."""
    conn = psycopg.connect(os.environ["GB_GRID_DATABASE_URL"], autocommit=True)
    with conn.cursor() as cur:
        cur.execute(f"TRUNCATE {', '.join(_TABLES)} RESTART IDENTITY CASCADE")
    try:
        yield conn
    finally:
        conn.close()
