from __future__ import annotations

import asyncio
from datetime import UTC, datetime, timedelta

import structlog

from .api.client import BMRSClient
from .config import POLL
from .db import connect, get_watermark
from .ingest.b1610 import ingest_b1610
from .ingest.boalf import ingest_boalf
from .ingest.fuelinst import ingest_fuelinst
from .ingest.system_prices import ingest_system_prices

log = structlog.get_logger(__name__)


def _utcnow() -> datetime:
    return datetime.now(UTC).replace(tzinfo=None)


async def _loop(name: str, interval: int, fn) -> None:
    while True:
        try:
            await asyncio.to_thread(fn)
        except Exception as exc:  # noqa: BLE001 — log and keep looping
            log.error("ingest_failed", dataset=name, error=str(exc))
        await asyncio.sleep(interval)


def _fuelinst_tick() -> None:
    conn = connect()
    try:
        with BMRSClient() as client:
            wm = get_watermark(conn, "fuelinst")
            now = _utcnow()
            start = wm or (now - timedelta(hours=2))
            ingest_fuelinst(conn, client, start, now)
    finally:
        conn.close()


def _boalf_tick() -> None:
    conn = connect()
    try:
        with BMRSClient() as client:
            now = _utcnow()
            start = now - timedelta(hours=4)  # rolling window catches revisions
            ingest_boalf(conn, client, start, now)
    finally:
        conn.close()


def _b1610_tick() -> None:
    conn = connect()
    try:
        with BMRSClient() as client:
            today = _utcnow().date()
            ingest_b1610(conn, client, today - timedelta(days=7), today)
    finally:
        conn.close()


def _system_prices_tick() -> None:
    conn = connect()
    try:
        with BMRSClient() as client:
            today = _utcnow().date()
            ingest_system_prices(conn, client, today - timedelta(days=1), today)
    finally:
        conn.close()


async def run_scheduler() -> None:
    log.info("scheduler_starting")
    await asyncio.gather(
        _loop("fuelinst", POLL.fuelinst_seconds, _fuelinst_tick),
        _loop("boalf", POLL.boalf_seconds, _boalf_tick),
        _loop("b1610", POLL.b1610_seconds, _b1610_tick),
        _loop("system_prices", POLL.system_prices_seconds, _system_prices_tick),
    )
