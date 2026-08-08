"""Coverage for the COPY-into-staging upsert path (batches >= COPY_MIN_ROWS)."""

from datetime import datetime, timedelta

from gb_grid.db import COPY_MIN_ROWS, upsert

CONFLICT = ["publish_time", "fuel_type"]
BASE = datetime(2026, 5, 1, 0, 0)


def _row(i: int, mw: float, fuel: str = "WIND") -> dict:
    return {
        "publish_time": BASE + timedelta(minutes=5 * i),
        "settlement_date": None,
        "settlement_period": 1,
        "fuel_type": fuel,
        "generation_mw": mw,
    }


def _rows(db) -> list[tuple]:
    with db.cursor() as cur:
        cur.execute(
            "SELECT publish_time, fuel_type, generation_mw FROM fuelinst "
            "ORDER BY publish_time, fuel_type"
        )
        return cur.fetchall()


def test_copy_path_inserts_all_rows(db):
    rows = [_row(i, float(i)) for i in range(COPY_MIN_ROWS + 10)]
    n = upsert(db, "fuelinst", rows, CONFLICT)
    assert n == len(rows)
    assert len(_rows(db)) == len(rows)


def test_copy_path_is_idempotent_and_updates(db):
    rows = [_row(i, float(i)) for i in range(COPY_MIN_ROWS + 10)]
    upsert(db, "fuelinst", rows, CONFLICT)
    bumped = [{**r, "generation_mw": r["generation_mw"] + 1000} for r in rows]
    upsert(db, "fuelinst", bumped, CONFLICT)

    stored = _rows(db)
    assert len(stored) == len(rows)
    assert stored[0][2] == 1000.0


def test_copy_path_dedupes_within_batch_last_wins(db):
    """A repeated key inside one batch must not abort the merge.

    ON CONFLICT DO UPDATE cannot touch the same row twice in one statement, so
    the staging select has to collapse duplicates first — keeping the last one,
    matching what the row-at-a-time executemany path would have left behind.
    """
    rows = [_row(i, float(i)) for i in range(COPY_MIN_ROWS)]
    rows.append(_row(0, 9999.0))  # duplicate key, arrives last

    n = upsert(db, "fuelinst", rows, CONFLICT)
    assert n == len(rows)

    stored = _rows(db)
    assert len(stored) == COPY_MIN_ROWS
    assert stored[0][2] == 9999.0


def test_copy_and_values_paths_agree(db):
    """The two paths must be behaviourally interchangeable."""
    small = [_row(i, float(i)) for i in range(10)]
    upsert(db, "fuelinst", small, CONFLICT)
    via_values = _rows(db)

    with db.cursor() as cur:
        cur.execute("TRUNCATE fuelinst")
    big = small + [_row(i, float(i)) for i in range(10, COPY_MIN_ROWS + 1)]
    upsert(db, "fuelinst", big, CONFLICT)

    assert _rows(db)[:10] == via_values
