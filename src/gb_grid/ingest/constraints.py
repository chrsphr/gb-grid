"""Day-ahead constraint flows and limits from the NESO open data portal.

The source is a single bulk CSV (full history, ~1 M rows) published daily on
weekdays.  We track a high-water mark so subsequent daily refreshes only upsert
the new rows rather than re-processing the entire file.
"""

from __future__ import annotations

import csv
import io
from datetime import datetime, timezone
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


def _parse_ts(raw: str) -> datetime:
    """Parse a GMT/BST wall-clock timestamp and return naive UTC."""
    dt = datetime.fromisoformat(raw).replace(tzinfo=_LONDON)
    return dt.astimezone(timezone.utc).replace(tzinfo=None)


def _fetch_csv() -> list[dict]:
    with httpx.Client(follow_redirects=True, timeout=120) as client:
        resp = client.get(CSV_URL)
        resp.raise_for_status()

    rows = []
    reader = csv.DictReader(io.StringIO(resp.content.decode("utf-8-sig")))
    for r in reader:
        try:
            rows.append({
                "constraint_group": r["Constraint Group"].strip(),
                "ts":               _parse_ts(r["Date (GMT/BST)"].strip()),
                "limit_mw":         float(r["Limit (MW)"]) if r["Limit (MW)"].strip() else None,
                "flow_mw":          float(r["Flow (MW)"]) if r["Flow (MW)"].strip() else None,
            })
        except (KeyError, ValueError):
            continue
    return rows


def ingest_constraints(conn: psycopg.Connection) -> int:
    with conn.cursor() as cur:
        cur.execute("SELECT MAX(ts) FROM constraints")
        row = cur.fetchone()
        last_ts: datetime | None = row[0]

    log.info("constraints_fetch_start", last_ts=last_ts)
    rows = _fetch_csv()

    if last_ts is not None:
        rows = [r for r in rows if r["ts"] > last_ts]

    if not rows:
        log.info("constraints_no_new_rows")
        return 0

    n = upsert(conn, TABLE, rows, CONFLICT)
    log.info("constraints_ingested", rows=n)
    return n
