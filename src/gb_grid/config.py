from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
DATA_DIR = Path(os.environ.get("GB_GRID_DATA_DIR", REPO_ROOT / "data"))
DB_PATH = Path(os.environ.get("GB_GRID_DB", DATA_DIR / "gb_grid.duckdb"))

API_BASE_URL = os.environ.get(
    "GB_GRID_API_BASE", "https://data.elexon.co.uk/bmrs/api/v1"
)
HTTP_TIMEOUT = float(os.environ.get("GB_GRID_HTTP_TIMEOUT", "60"))


@dataclass(frozen=True)
class PollConfig:
    fuelinst_seconds: int = 300
    boalf_seconds: int = 300
    b1610_seconds: int = 1800
    system_prices_seconds: int = 3600


POLL = PollConfig()
