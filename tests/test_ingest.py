from datetime import datetime

import httpx
import pytest

from gb_grid.api.client import BMRSClient
from gb_grid.db import get_watermark
from gb_grid.ingest.fuelinst import ingest_fuelinst


@pytest.fixture
def client():
    return BMRSClient(client=httpx.Client(base_url="https://example.test"))


def test_ingest_fuelinst_writes_rows_and_watermark(httpx_mock, client, db):
    httpx_mock.add_response(
        json={
            "data": [
                {
                    "publishTime": "2026-05-01T00:05:00Z",
                    "settlementDate": "2026-05-01",
                    "settlementPeriod": 1,
                    "fuelType": "WIND",
                    "generation": 1000.0,
                },
                {
                    "publishTime": "2026-05-01T00:10:00Z",
                    "settlementDate": "2026-05-01",
                    "settlementPeriod": 1,
                    "fuelType": "GAS",
                    "generation": 7000.0,
                },
            ]
        }
    )
    n = ingest_fuelinst(
        db, client, datetime(2026, 5, 1, 0, 0), datetime(2026, 5, 1, 1, 0)
    )
    assert n == 2
    with db.cursor() as cur:
        cur.execute("SELECT count(*) FROM fuelinst")
        assert cur.fetchone()[0] == 2
    wm = get_watermark(db, "fuelinst")
    assert wm is not None
