from __future__ import annotations

from datetime import datetime
from typing import Any

import httpx
from tenacity import (
    retry,
    retry_if_exception,
    stop_after_attempt,
    wait_exponential,
)

from ..config import API_BASE_URL, HTTP_TIMEOUT


def _is_retryable(exc: BaseException) -> bool:
    """Only transport failures and transient statuses are worth another attempt.

    A 400/404 means the request itself is wrong (bad window, dataset absent for
    that date) and will fail identically five times over, so retrying it just
    stalls a backfill for ~30s per bad chunk before surfacing the same error.
    """
    if isinstance(exc, httpx.TransportError):  # covers TimeoutException
        return True
    if isinstance(exc, httpx.HTTPStatusError):
        status = exc.response.status_code
        return status == 429 or status >= 500
    return False


def _iso(dt: datetime) -> str:
    # BMRS expects ISO8601 with seconds; treat naive datetimes as UTC.
    if dt.tzinfo is None:
        return dt.strftime("%Y-%m-%dT%H:%M:%SZ")
    return dt.strftime("%Y-%m-%dT%H:%M:%S%z")


class BMRSClient:
    """Thin synchronous client over the Elexon BMRS Insights API.

    The Insights API is open (no auth). Responses are JSON; most dataset endpoints
    return either a top-level list or `{"data": [...]}`.
    """

    def __init__(
        self,
        base_url: str = API_BASE_URL,
        timeout: float = HTTP_TIMEOUT,
        client: httpx.Client | None = None,
    ) -> None:
        self._owns_client = client is None
        self._client = client or httpx.Client(
            base_url=base_url,
            timeout=timeout,
            headers={"accept": "application/json"},
        )

    def close(self) -> None:
        if self._owns_client:
            self._client.close()

    def __enter__(self) -> BMRSClient:
        return self

    def __exit__(self, *exc: object) -> None:
        self.close()

    @retry(
        reraise=True,
        stop=stop_after_attempt(5),
        wait=wait_exponential(multiplier=1, min=1, max=30),
        retry=retry_if_exception(_is_retryable),
    )
    def get(self, path: str, params: dict[str, Any] | None = None) -> Any:
        resp = self._client.get(path, params=params)
        resp.raise_for_status()  # 429/5xx retry, other 4xx fail fast
        return resp.json()

    @staticmethod
    def extract_data(payload: Any) -> list[dict[str, Any]]:
        """Normalise BMRS payload to a list of records."""
        if isinstance(payload, list):
            return payload
        if isinstance(payload, dict):
            for key in ("data", "Data", "results", "items"):
                if key in payload and isinstance(payload[key], list):
                    return payload[key]
        return []
