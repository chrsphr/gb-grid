from datetime import datetime

from gb_grid.db import connect, get_watermark, set_watermark, upsert


def test_migrate_creates_tables():
    conn = connect(":memory:")
    tables = {r[0] for r in conn.execute("SHOW TABLES").fetchall()}
    assert {"fuelinst", "b1610", "boalf", "system_prices", "ingest_watermark"} <= tables


def test_upsert_idempotent():
    conn = connect(":memory:")
    rows = [
        {
            "publish_time": datetime(2026, 5, 1, 12, 0),
            "settlement_date": None,
            "settlement_period": 25,
            "fuel_type": "WIND",
            "generation_mw": 5000.0,
        }
    ]
    upsert(conn, "fuelinst", rows, ["publish_time", "fuel_type"])
    upsert(conn, "fuelinst", rows, ["publish_time", "fuel_type"])
    n = conn.execute("SELECT count(*) FROM fuelinst").fetchone()[0]
    assert n == 1


def test_upsert_updates_value_on_conflict():
    conn = connect(":memory:")
    base = {
        "publish_time": datetime(2026, 5, 1, 12, 0),
        "settlement_date": None,
        "settlement_period": 25,
        "fuel_type": "WIND",
        "generation_mw": 1000.0,
    }
    upsert(conn, "fuelinst", [base], ["publish_time", "fuel_type"])
    upsert(
        conn, "fuelinst", [{**base, "generation_mw": 2000.0}], ["publish_time", "fuel_type"]
    )
    val = conn.execute("SELECT generation_mw FROM fuelinst").fetchone()[0]
    assert val == 2000.0


def test_watermark_roundtrip():
    conn = connect(":memory:")
    ts = datetime(2026, 5, 1, 12, 30)
    set_watermark(conn, "fuelinst", ts)
    assert get_watermark(conn, "fuelinst") == ts
    set_watermark(conn, "fuelinst", datetime(2026, 5, 2, 0, 0))
    assert get_watermark(conn, "fuelinst") == datetime(2026, 5, 2, 0, 0)
