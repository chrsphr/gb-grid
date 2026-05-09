from __future__ import annotations

import asyncio
import os
from datetime import date, datetime, time
from typing import Annotated

import structlog
import typer

from .api.client import BMRSClient
from .config import database_url
from .db import connect, table_stats
from .db import migrate as run_migrate
from .ingest import DATASETS
from .ingest.b1610 import ingest_b1610
from .ingest.boalf import ingest_boalf
from .ingest.fuelinst import ingest_fuelinst
from .ingest.mels import ingest_mels
from .ingest.pn import ingest_pn
from .ingest.system_prices import ingest_system_prices

structlog.configure(
    processors=[
        structlog.processors.TimeStamper(fmt="iso"),
        structlog.processors.add_log_level,
        structlog.processors.KeyValueRenderer(),
    ]
)

app = typer.Typer(help="GB power grid energy database CLI.", no_args_is_help=True)


def _parse_date(s: str) -> date:
    return datetime.strptime(s, "%Y-%m-%d").date()


@app.command()
def migrate() -> None:
    """Apply any pending database migrations."""
    n = run_migrate()
    typer.echo(f"applied {n} migration(s)")


@app.command()
def backfill(
    from_: Annotated[str, typer.Option("--from", help="YYYY-MM-DD inclusive")],
    to: Annotated[str, typer.Option("--to", help="YYYY-MM-DD inclusive")],
    dataset: Annotated[
        list[str] | None,
        typer.Option("--dataset", "-d", help="Repeatable. Default: all."),
    ] = None,
) -> None:
    """Backfill historical data into the database."""
    start_d = _parse_date(from_)
    end_d = _parse_date(to)
    start_dt = datetime.combine(start_d, time.min)
    end_dt = datetime.combine(end_d, time.max)

    selected = dataset or list(DATASETS.keys())
    unknown = [d for d in selected if d not in DATASETS]
    if unknown:
        raise typer.BadParameter(f"unknown datasets: {unknown}")

    conn = connect()
    try:
        with BMRSClient() as client:
            for d in selected:
                typer.echo(f"-> {d}: {start_d} .. {end_d}")
                if d == "fuelinst":
                    ingest_fuelinst(conn, client, start_dt, end_dt)
                elif d == "boalf":
                    ingest_boalf(conn, client, start_dt, end_dt)
                elif d == "b1610":
                    ingest_b1610(conn, client, start_d, end_d)
                elif d == "pn":
                    ingest_pn(conn, client, start_dt, end_dt)
                elif d == "mels":
                    ingest_mels(conn, client, start_dt, end_dt)
                elif d == "system_prices":
                    ingest_system_prices(conn, client, start_d, end_d)
    finally:
        conn.close()
    typer.echo("done.")


@app.command("materialize-dispatch")
def materialize_dispatch_cmd(
    from_: Annotated[str, typer.Option("--from", help="YYYY-MM-DD inclusive")],
    to: Annotated[str, typer.Option("--to", help="YYYY-MM-DD inclusive")],
    bmu: Annotated[
        list[str] | None,
        typer.Option("--bmu", "-b", help="Repeatable. Default: all BMUs with PN data."),
    ] = None,
    workers: Annotated[
        int | None,
        typer.Option("--workers", "-j", help="Process pool size. Default: ncpu-1."),
    ] = None,
) -> None:
    """Recompute bmu_dispatch (only BMUs with BOA acceptances in the window)."""
    from .materialize import materialize_dispatch

    start_dt = datetime.combine(_parse_date(from_), time.min)
    end_dt = datetime.combine(_parse_date(to), time.max)
    conn = connect()
    try:
        n = materialize_dispatch(conn, start_dt, end_dt, bmus=bmu, workers=workers)
    finally:
        conn.close()
    typer.echo(f"wrote {n} rows")


@app.command()
def run() -> None:
    """Start the always-on async scheduler."""
    from .scheduler import run_scheduler

    asyncio.run(run_scheduler())


@app.command()
def status() -> None:
    """Print row counts and latest timestamps per table."""
    conn = connect()
    try:
        rows = [
            table_stats(conn, "fuelinst", "publish_time"),
            table_stats(conn, "b1610", "settlement_date"),
            table_stats(conn, "boalf", "time_from"),
            table_stats(conn, "pn", "time_from"),
            table_stats(conn, "mels", "time_from"),
            table_stats(conn, "system_prices", "settlement_date"),
        ]
        with conn.cursor() as cur:
            cur.execute(
                "SELECT dataset, last_ts, updated_at FROM ingest_watermark ORDER BY dataset"
            )
            wm = cur.fetchall()
    finally:
        conn.close()

    typer.echo(f"db: {database_url()}")
    typer.echo("table          rows         latest")
    for r in rows:
        typer.echo(f"{r['table']:<14} {r['rows']:<12} {r['latest']}")
    typer.echo("")
    typer.echo("watermarks:")
    for dataset, last_ts, updated_at in wm:
        typer.echo(f"  {dataset:<14} last_ts={last_ts}  updated_at={updated_at}")


@app.command()
def sql() -> None:
    """Open psql against the configured database."""
    url = database_url()
    if not url:
        raise typer.BadParameter("GB_GRID_DATABASE_URL is not set")
    os.execvp("psql", ["psql", url])


if __name__ == "__main__":
    app()
