from datetime import datetime

from gb_grid.ingest.constraints import CSV_URL, _parse_ts, ingest_constraints

CSV = (
    "﻿Constraint Group,Date (GMT/BST),Limit (MW),Flow (MW)\r\n"
    "B6,2026-05-01 00:00:00,1000,900\r\n"
    "B6,2026-05-01 00:30:00,1000,\r\n"
    "SCOTEX,2026-05-01 00:00:00,2000,1500\r\n"
)


def test_ingest_parses_and_stores(httpx_mock, db):
    httpx_mock.add_response(url=CSV_URL, text=CSV, headers={"ETag": '"v1"'})

    assert ingest_constraints(db) == 3
    with db.cursor() as cur:
        cur.execute(
            "SELECT constraint_group, limit_mw, flow_mw FROM constraints "
            "ORDER BY ts, constraint_group"
        )
        rows = cur.fetchall()
    assert rows[0] == ("B6", 1000.0, 900.0)
    assert rows[2][2] is None  # empty Flow (MW) -> NULL


def test_bom_does_not_corrupt_first_column(httpx_mock, db):
    httpx_mock.add_response(url=CSV_URL, text=CSV)
    assert ingest_constraints(db) == 3


def test_etag_is_stored_and_replayed(httpx_mock, db):
    httpx_mock.add_response(url=CSV_URL, text=CSV, headers={"ETag": '"v1"'})
    ingest_constraints(db)

    with db.cursor() as cur:
        cur.execute("SELECT etag FROM ingest_http_cache WHERE url = %s", (CSV_URL,))
        assert cur.fetchone()[0] == '"v1"'

    # Unchanged file: the server answers 304 and we skip the parse entirely.
    httpx_mock.add_response(url=CSV_URL, status_code=304)
    assert ingest_constraints(db) == 0

    sent = httpx_mock.get_requests()[-1]
    assert sent.headers["If-None-Match"] == '"v1"'


def test_rows_at_or_below_watermark_are_skipped(httpx_mock, db):
    httpx_mock.add_response(url=CSV_URL, text=CSV)
    ingest_constraints(db)

    # Same file again (no validator match): only rows newer than MAX(ts) survive,
    # and here nothing is newer.
    httpx_mock.add_response(url=CSV_URL, text=CSV)
    assert ingest_constraints(db) == 0


def test_parse_ts_converts_bst_to_utc():
    assert _parse_ts("2026-07-01 12:00:00") == datetime(2026, 7, 1, 11, 0)  # BST
    assert _parse_ts("2026-01-01 12:00:00") == datetime(2026, 1, 1, 12, 0)  # GMT
