from __future__ import annotations

from datetime import date, datetime
from typing import Any

from .client import BMRSClient, _iso


def _get(d: dict[str, Any], *keys: str) -> Any:
    for k in keys:
        if k in d and d[k] is not None:
            return d[k]
    return None


def _parse_dt(v: Any) -> datetime | None:
    if v is None:
        return None
    if isinstance(v, datetime):
        return v
    s = str(v).replace("Z", "+00:00")
    try:
        return datetime.fromisoformat(s)
    except ValueError:
        return None


def fetch_fuelinst(
    client: BMRSClient, start: datetime, end: datetime
) -> list[dict[str, Any]]:
    """Generation by fuel type — 5 min cadence."""
    payload = client.get(
        "/datasets/FUELINST/stream",
        params={
            "publishDateTimeFrom": _iso(start),
            "publishDateTimeTo": _iso(end),
            "format": "json",
        },
    )
    out: list[dict[str, Any]] = []
    for r in client.extract_data(payload):
        publish = _parse_dt(_get(r, "publishTime", "PublishTime", "startTime", "StartTime"))
        if publish is None:
            continue
        out.append(
            {
                "publish_time": publish,
                "settlement_date": _parse_dt(
                    _get(r, "settlementDate", "SettlementDate")
                ).date()
                if _get(r, "settlementDate", "SettlementDate")
                else None,
                "settlement_period": _get(r, "settlementPeriod", "SettlementPeriod"),
                "fuel_type": _get(r, "fuelType", "FuelType", "psrType"),
                "generation_mw": _get(r, "generation", "Generation", "quantity"),
            }
        )
    return out


def fetch_b1610(
    client: BMRSClient, start: date, end: date
) -> list[dict[str, Any]]:
    """Per-BMU actual generation, half-hourly settlement periods.

    The API's ``from``/``to`` are interpreted as instants — passing a bare date
    only returns settlement period 1. Always send the full-day datetime span.
    """
    start_dt = datetime.combine(start, datetime.min.time())
    end_dt = datetime.combine(end, datetime.max.time().replace(microsecond=0))
    payload = client.get(
        "/datasets/B1610/stream",
        params={
            "from": _iso(start_dt),
            "to": _iso(end_dt),
            "format": "json",
        },
    )
    out: list[dict[str, Any]] = []
    for r in client.extract_data(payload):
        sd = _parse_dt(_get(r, "settlementDate", "SettlementDate"))
        bm_unit = _get(r, "bmUnit", "BmUnit", "bMUnitId")
        period = _get(r, "settlementPeriod", "SettlementPeriod")
        if sd is None or bm_unit is None or period is None:
            continue
        out.append(
            {
                "settlement_date": sd.date(),
                "settlement_period": period,
                "bm_unit": bm_unit,
                "ngc_bm_unit": _get(
                    r, "nationalGridBmUnitId", "nationalGridBmUnit", "ngcBmUnit"
                ),
                "quantity_mwh": _get(r, "quantity", "Quantity", "generation"),
            }
        )
    return out


def fetch_boalf(
    client: BMRSClient, start: datetime, end: datetime
) -> list[dict[str, Any]]:
    """Bid-Offer Acceptance Level Flagged — balancing acceptances."""
    payload = client.get(
        "/datasets/BOALF/stream",
        params={
            "from": _iso(start),
            "to": _iso(end),
            "format": "json",
        },
    )
    out: list[dict[str, Any]] = []
    for r in client.extract_data(payload):
        time_from = _parse_dt(_get(r, "timeFrom", "TimeFrom"))
        accept_id = _get(r, "acceptanceNumber", "AcceptanceNumber", "acceptanceId")
        if time_from is None or accept_id is None:
            continue
        out.append(
            {
                "acceptance_id": int(accept_id),
                "bm_unit": _get(r, "bmUnit", "BmUnit"),
                "ngc_bm_unit": _get(r, "nationalGridBmUnit", "NationalGridBmUnit"),
                "acceptance_time": _parse_dt(_get(r, "acceptanceTime", "AcceptanceTime")),
                "time_from": time_from,
                "time_to": _parse_dt(_get(r, "timeTo", "TimeTo")),
                "level_from": _get(r, "levelFrom", "LevelFrom"),
                "level_to": _get(r, "levelTo", "LevelTo"),
                "deemed_bo_flag": _get(r, "deemedBoFlag", "DeemedBoFlag"),
                "so_flag": _get(r, "soFlag", "SoFlag"),
            }
        )
    return out


def fetch_pn(
    client: BMRSClient, start: datetime, end: datetime
) -> list[dict[str, Any]]:
    """Physical Notifications — each BMU's planned MW level (FPN)."""
    payload = client.get(
        "/datasets/PN/stream",
        params={"from": _iso(start), "to": _iso(end), "format": "json"},
    )
    out: list[dict[str, Any]] = []
    for r in client.extract_data(payload):
        ngc = _get(r, "nationalGridBmUnit", "NationalGridBmUnit")
        time_from = _parse_dt(_get(r, "timeFrom", "TimeFrom"))
        if ngc is None or time_from is None:
            continue
        sd = _parse_dt(_get(r, "settlementDate", "SettlementDate"))
        out.append(
            {
                "national_grid_bm_unit": ngc,
                "bm_unit": _get(r, "bmUnit", "BmUnit"),
                "settlement_date": sd.date() if sd else None,
                "settlement_period": _get(r, "settlementPeriod", "SettlementPeriod"),
                "time_from": time_from,
                "time_to": _parse_dt(_get(r, "timeTo", "TimeTo")),
                "level_from": _get(r, "levelFrom", "LevelFrom"),
                "level_to": _get(r, "levelTo", "LevelTo"),
            }
        )
    return out


def fetch_mels(
    client: BMRSClient, start: datetime, end: datetime
) -> list[dict[str, Any]]:
    """Maximum Export Limit (MELS) — per-BMU export caps with revisions."""
    payload = client.get(
        "/datasets/MELS/stream",
        params={"from": _iso(start), "to": _iso(end), "format": "json"},
    )
    out: list[dict[str, Any]] = []
    for r in client.extract_data(payload):
        ngc = _get(r, "nationalGridBmUnit", "NationalGridBmUnit")
        time_from = _parse_dt(_get(r, "timeFrom", "TimeFrom"))
        seq = _get(r, "notificationSequence", "NotificationSequence")
        if ngc is None or time_from is None or seq is None:
            continue
        sd = _parse_dt(_get(r, "settlementDate", "SettlementDate"))
        out.append(
            {
                "national_grid_bm_unit": ngc,
                "bm_unit": _get(r, "bmUnit", "BmUnit"),
                "settlement_date": sd.date() if sd else None,
                "settlement_period": _get(r, "settlementPeriod", "SettlementPeriod"),
                "time_from": time_from,
                "time_to": _parse_dt(_get(r, "timeTo", "TimeTo")),
                "level_from": _get(r, "levelFrom", "LevelFrom"),
                "level_to": _get(r, "levelTo", "LevelTo"),
                "notification_time": _parse_dt(
                    _get(r, "notificationTime", "NotificationTime")
                ),
                "notification_sequence": int(seq),
            }
        )
    return out


def fetch_system_prices(client: BMRSClient, day: date) -> list[dict[str, Any]]:
    """System imbalance prices for one settlement date."""
    payload = client.get(
        f"/balancing/settlement/system-prices/{day.isoformat()}",
        params={"format": "json"},
    )
    out: list[dict[str, Any]] = []
    for r in client.extract_data(payload):
        period = _get(r, "settlementPeriod", "SettlementPeriod")
        if period is None:
            continue
        out.append(
            {
                "settlement_date": day,
                "settlement_period": period,
                "system_sell_price": _get(r, "systemSellPrice", "SystemSellPrice"),
                "system_buy_price": _get(r, "systemBuyPrice", "SystemBuyPrice"),
                "net_imbalance_volume": _get(
                    r, "netImbalanceVolume", "NetImbalanceVolume"
                ),
            }
        )
    return out
