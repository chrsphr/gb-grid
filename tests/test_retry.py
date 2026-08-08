import httpx
import pytest

from gb_grid.api.client import BMRSClient, _is_retryable


@pytest.fixture
def client():
    return BMRSClient(client=httpx.Client(base_url="https://example.test"))


def _status_error(code: int) -> httpx.HTTPStatusError:
    request = httpx.Request("GET", "https://example.test/x")
    response = httpx.Response(code, request=request)
    return httpx.HTTPStatusError("boom", request=request, response=response)


@pytest.mark.parametrize("code", [429, 500, 502, 503])
def test_transient_statuses_are_retryable(code):
    assert _is_retryable(_status_error(code))


@pytest.mark.parametrize("code", [400, 401, 403, 404, 422])
def test_permanent_statuses_are_not_retryable(code):
    assert not _is_retryable(_status_error(code))


def test_transport_and_timeout_errors_are_retryable():
    request = httpx.Request("GET", "https://example.test/x")
    assert _is_retryable(httpx.ConnectError("down", request=request))
    assert _is_retryable(httpx.ReadTimeout("slow", request=request))


def test_get_does_not_retry_a_400(httpx_mock, client):
    """A bad window must surface immediately, not after five backed-off attempts."""
    httpx_mock.add_response(status_code=400)

    with pytest.raises(httpx.HTTPStatusError):
        client.get("/datasets/PN/stream")

    assert len(httpx_mock.get_requests()) == 1


def test_get_retries_a_500_then_succeeds(httpx_mock, client):
    httpx_mock.add_response(status_code=500)
    httpx_mock.add_response(json={"data": [{"a": 1}]})

    assert client.get("/datasets/PN/stream") == {"data": [{"a": 1}]}
    assert len(httpx_mock.get_requests()) == 2
