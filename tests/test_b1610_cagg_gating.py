"""The b1610 tick must not redo work the cagg refresh policies already do."""

from datetime import UTC, date, datetime, timedelta

import httpx
import pytest

from gb_grid.api.client import BMRSClient
from gb_grid.ingest import b1610 as mod


@pytest.fixture
def client():
    return BMRSClient(client=httpx.Client(base_url="https://example.test"))


@pytest.fixture
def refreshes(monkeypatch):
    """Record refresh_caggs calls instead of running them."""
    calls = []
    monkeypatch.setattr(
        mod, "refresh_caggs", lambda conn, caggs, start, end: calls.append((start, end))
    )
    return calls


def _payload(day: date) -> dict:
    return {
        "data": [
            {
                "settlementDate": day.isoformat(),
                "settlementPeriod": 1,
                "bmUnit": "T_TEST-1",
                "nationalGridBmUnit": "TEST1",
                "quantity": 100.0,
            }
        ]
    }


def test_recent_window_skips_refresh(httpx_mock, client, db, refreshes):
    """Inside the policies' 30-day reach: the background policy has it covered."""
    today = datetime.now(UTC).date()
    start, end = today - timedelta(days=14), today - timedelta(days=3)
    for i in range((end - start).days + 1):
        httpx_mock.add_response(json=_payload(start + timedelta(days=i)))

    assert mod.ingest_b1610(db, client, start, end) > 0
    assert refreshes == []


def test_historical_window_refreshes_only_the_uncovered_part(
    httpx_mock, client, db, refreshes
):
    """A backfill straddling the boundary refreshes history, not the recent tail."""
    today = datetime.now(UTC).date()
    start = today - timedelta(days=60)
    end = today - timedelta(days=3)
    for i in range((end - start).days + 1):
        httpx_mock.add_response(json=_payload(start + timedelta(days=i)))

    mod.ingest_b1610(db, client, start, end)

    assert len(refreshes) == 1
    r_start, r_end = refreshes[0]
    policy_floor = today - mod.CAGG_POLICY_REACH
    assert r_start == start
    assert r_end == policy_floor + timedelta(days=1)
    assert r_end < end  # the recent tail is left to the policy


def test_no_rows_means_no_refresh(httpx_mock, client, db, refreshes):
    today = datetime.now(UTC).date()
    start, end = today - timedelta(days=60), today - timedelta(days=59)
    httpx_mock.add_response(json={"data": []})
    httpx_mock.add_response(json={"data": []})

    assert mod.ingest_b1610(db, client, start, end) == 0
    assert refreshes == []
