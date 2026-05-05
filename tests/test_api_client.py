from datetime import date, datetime

import httpx
import pytest

from gb_grid.api.client import BMRSClient
from gb_grid.api.endpoints import fetch_fuelinst, fetch_system_prices


@pytest.fixture
def client():
    return BMRSClient(client=httpx.Client(base_url="https://example.test"))


def test_extract_data_handles_list_and_dict():
    assert BMRSClient.extract_data([{"a": 1}]) == [{"a": 1}]
    assert BMRSClient.extract_data({"data": [{"a": 1}]}) == [{"a": 1}]
    assert BMRSClient.extract_data({"results": [{"a": 1}]}) == [{"a": 1}]
    assert BMRSClient.extract_data({"unrelated": 5}) == []


def test_fetch_fuelinst_shapes_rows(httpx_mock, client):
    httpx_mock.add_response(
        url=httpx.URL(
            "https://example.test/datasets/FUELINST/stream",
            params={
                "publishDateTimeFrom": "2026-05-01T00:00:00Z",
                "publishDateTimeTo": "2026-05-01T01:00:00Z",
                "format": "json",
            },
        ),
        json={
            "data": [
                {
                    "publishTime": "2026-05-01T00:05:00Z",
                    "settlementDate": "2026-05-01",
                    "settlementPeriod": 1,
                    "fuelType": "WIND",
                    "generation": 12345.6,
                }
            ]
        },
    )
    rows = fetch_fuelinst(
        client, datetime(2026, 5, 1, 0, 0), datetime(2026, 5, 1, 1, 0)
    )
    assert len(rows) == 1
    r = rows[0]
    assert r["fuel_type"] == "WIND"
    assert r["generation_mw"] == 12345.6
    assert r["settlement_period"] == 1
    assert r["publish_time"].year == 2026


def test_fetch_system_prices_shapes_rows(httpx_mock, client):
    httpx_mock.add_response(
        url=httpx.URL(
            "https://example.test/balancing/settlement/system-prices/2026-05-01",
            params={"format": "json"},
        ),
        json={
            "data": [
                {
                    "settlementPeriod": 1,
                    "systemSellPrice": 50.0,
                    "systemBuyPrice": 55.0,
                    "netImbalanceVolume": -10.5,
                }
            ]
        },
    )
    rows = fetch_system_prices(client, date(2026, 5, 1))
    assert rows[0]["system_sell_price"] == 50.0
    assert rows[0]["settlement_date"] == date(2026, 5, 1)
