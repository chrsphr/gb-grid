from datetime import datetime

from gb_grid.db import get_watermark, set_watermark, upsert


def test_migrate_creates_tables(db):
    with db.cursor() as cur:
        cur.execute(
            "SELECT tablename FROM pg_tables WHERE schemaname = 'public'"
        )
        tables = {r[0] for r in cur.fetchall()}
    assert {"fuelinst", "b1610", "boalf", "system_prices", "ingest_watermark"} <= tables


def test_upsert_idempotent(db):
    rows = [
        {
            "publish_time": datetime(2026, 5, 1, 12, 0),
            "settlement_date": None,
            "settlement_period": 25,
            "fuel_type": "WIND",
            "generation_mw": 5000.0,
        }
    ]
    upsert(db, "fuelinst", rows, ["publish_time", "fuel_type"])
    upsert(db, "fuelinst", rows, ["publish_time", "fuel_type"])
    with db.cursor() as cur:
        cur.execute("SELECT count(*) FROM fuelinst")
        n = cur.fetchone()[0]
    assert n == 1


def test_upsert_updates_value_on_conflict(db):
    base = {
        "publish_time": datetime(2026, 5, 1, 12, 0),
        "settlement_date": None,
        "settlement_period": 25,
        "fuel_type": "WIND",
        "generation_mw": 1000.0,
    }
    upsert(db, "fuelinst", [base], ["publish_time", "fuel_type"])
    upsert(
        db, "fuelinst", [{**base, "generation_mw": 2000.0}], ["publish_time", "fuel_type"]
    )
    with db.cursor() as cur:
        cur.execute("SELECT generation_mw FROM fuelinst")
        val = cur.fetchone()[0]
    assert val == 2000.0


def test_watermark_roundtrip(db):
    ts = datetime(2026, 5, 1, 12, 30)
    set_watermark(db, "fuelinst", ts)
    assert get_watermark(db, "fuelinst") == ts
    set_watermark(db, "fuelinst", datetime(2026, 5, 2, 0, 0))
    assert get_watermark(db, "fuelinst") == datetime(2026, 5, 2, 0, 0)
