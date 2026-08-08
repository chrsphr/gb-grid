"""Day-ahead constraint flows and limits from the NESO open data portal.

The source is a single bulk CSV (full history, ~1 M rows) published daily on
weekdays.  We track a high-water mark so subsequent daily refreshes only upsert
the new rows rather than re-processing the entire file.
"""

from __future__ import annotations

import csv
from collections.abc import Iterator
from datetime import UTC, datetime
from zoneinfo import ZoneInfo

import httpx
import psycopg
import structlog

from ..db import upsert

log = structlog.get_logger(__name__)

CSV_URL = (
    "https://api.neso.energy/dataset/cf3cbc92-2d5d-4c2b-bd29-e11a21070b26"
    "/resource/38a18ec1-9e40-465d-93fb-301e80fd1352"
    "/download/day-ahead-constraints-limits-and-flow-output-v1.5.csv"
)

TABLE = "constraints"
CONFLICT = ["constraint_group", "ts"]
_LONDON = ZoneInfo("Europe/London")


_TS_CACHE: dict[str, datetime] = {}


def _parse_ts(raw: str) -> datetime:
    """Parse a GMT/BST wall-clock timestamp and return naive UTC.

    Memoised on the raw string: the file carries one row per constraint group
    per half hour, so each timestamp repeats across every group in that period
    and the tz conversion is the dominant per-row cost.
    """
    hit = _TS_CACHE.get(raw)
    if hit is None:
        dt = datetime.fromisoformat(raw).replace(tzinfo=_LONDON)
        hit = _TS_CACHE[raw] = dt.astimezone(UTC).replace(tzinfo=None)
    return hit


def _float_or_none(raw: str) -> float | None:
    return float(raw) if raw.strip() else None


def _strip_bom(lines: Iterator[str]) -> Iterator[str]:
    """Drop the UTF-8 BOM so the first CSV header name parses cleanly."""
    for i, line in enumerate(lines):
        yield line.lstrip("\ufeff") if i == 0 else line


def _get_validators(conn: psycopg.Connection) -> tuple[str | None, str | None]:
    with conn.cursor() as cur:
        cur.execute(
            "SELECT etag, last_modified FROM ingest_http_cache WHERE url = %s",
            (CSV_URL,),
        )
        row = cur.fetchone()
    return (row[0], row[1]) if row else (None, None)


def _save_validators(
    conn: psycopg.Connection, etag: str | None, last_modified: str | None
) -> None:
    with conn.cursor() as cur:
        cur.execute(
            "INSERT INTO ingest_http_cache(url, etag, last_modified, fetched_at) "
            "VALUES (%s, %s, %s, CURRENT_TIMESTAMP) "
            "ON CONFLICT (url) DO UPDATE SET "
            "  etag = EXCLUDED.etag, last_modified = EXCLUDED.last_modified, "
            "  fetched_at = EXCLUDED.fetched_at",
            (CSV_URL, etag, last_modified),
        )


def _stream_rows(
    conn: psycopg.Connection, last_ts: datetime | None
) -> list[dict] | None:
    """Download and parse the CSV. ``None`` means the file is unchanged (304).

    The response is streamed line by line rather than buffered whole — holding
    the raw bytes, the decoded string and the parsed rows at once costs several
    hundred MB on a file this size. Rows at or below ``last_ts`` are dropped
    before the dict and float parsing, which is nearly all of them on a daily
    refresh.
    """
    etag, last_modified = _get_validators(conn)
    headers = {}
    if etag:
        headers["If-None-Match"] = etag
    if last_modified:
        headers["If-Modified-Since"] = last_modified

    rows: list[dict] = []
    with (
        httpx.Client(follow_redirects=True, timeout=120) as client,
        client.stream("GET", CSV_URL, headers=headers) as resp,
    ):
        if resp.status_code == 304:
            return None
        resp.raise_for_status()
        reader = csv.DictReader(_strip_bom(resp.iter_lines()))
        for r in reader:
            try:
                ts = _parse_ts(r["Date (GMT/BST)"].strip())
                if last_ts is not None and ts <= last_ts:
                    continue
                rows.append({
                    "constraint_group": r["Constraint Group"].strip(),
                    "ts":               ts,
                    "limit_mw":         _float_or_none(r["Limit (MW)"]),
                    "flow_mw":          _float_or_none(r["Flow (MW)"]),
                })
            except (KeyError, ValueError, AttributeError):
                continue
        new_etag = resp.headers.get("etag")
        new_last_modified = resp.headers.get("last-modified")

    _save_validators(conn, new_etag, new_last_modified)
    return rows


def ingest_constraints(conn: psycopg.Connection) -> int:
    with conn.cursor() as cur:
        cur.execute("SELECT MAX(ts) FROM constraints")
        row = cur.fetchone()
        last_ts: datetime | None = row[0]

    log.info("constraints_fetch_start", last_ts=last_ts)
    rows = _stream_rows(conn, last_ts)

    if rows is None:
        log.info("constraints_not_modified")
        return 0
    if not rows:
        log.info("constraints_no_new_rows")
        return 0

    n = upsert(conn, TABLE, rows, CONFLICT)
    log.info("constraints_ingested", rows=n)
    return n
