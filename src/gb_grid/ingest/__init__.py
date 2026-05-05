from .b1610 import ingest_b1610
from .boalf import ingest_boalf
from .fuelinst import ingest_fuelinst
from .system_prices import ingest_system_prices

DATASETS = {
    "fuelinst": ingest_fuelinst,
    "b1610": ingest_b1610,
    "boalf": ingest_boalf,
    "system_prices": ingest_system_prices,
}

__all__ = ["DATASETS", "ingest_fuelinst", "ingest_b1610", "ingest_boalf", "ingest_system_prices"]
