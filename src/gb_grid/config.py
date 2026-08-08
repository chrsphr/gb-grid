from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]

def database_url() -> str | None:
    return os.environ.get("GB_GRID_DATABASE_URL")

API_BASE_URL = os.environ.get(
    "GB_GRID_API_BASE", "https://data.elexon.co.uk/bmrs/api/v1"
)
HTTP_TIMEOUT = float(os.environ.get("GB_GRID_HTTP_TIMEOUT", "60"))


DEFAULT_POLL_SECONDS = 3600

POLL_DATASETS = (
    "fuelinst",
    "boalf",
    "b1610",
    "pn",
    "mels",
    "system_prices",
    "materialize",
    "materialize_daily",
)


def _poll_seconds(dataset: str) -> int:
    """Interval for one scheduler loop.

    ``GB_GRID_POLL_<DATASET>_SECONDS`` overrides a single loop;
    ``GB_GRID_POLL_SECONDS`` moves the baseline for all of them.
    """
    raw = os.environ.get(f"GB_GRID_POLL_{dataset.upper()}_SECONDS") or os.environ.get(
        "GB_GRID_POLL_SECONDS"
    )
    if raw is None:
        return DEFAULT_POLL_SECONDS
    seconds = int(raw)
    if seconds <= 0:
        raise ValueError(f"poll interval for {dataset} must be positive, got {seconds}")
    return seconds


@dataclass(frozen=True)
class PollConfig:
    fuelinst_seconds: int
    boalf_seconds: int
    b1610_seconds: int
    pn_seconds: int
    mels_seconds: int
    system_prices_seconds: int
    materialize_seconds: int
    materialize_daily_seconds: int

    @classmethod
    def from_env(cls) -> PollConfig:
        return cls(**{f"{d}_seconds": _poll_seconds(d) for d in POLL_DATASETS})


POLL = PollConfig.from_env()
