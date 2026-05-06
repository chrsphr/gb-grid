from datetime import datetime

from gb_grid.analytics import bmu_dispatch
from gb_grid.db import upsert


def _seed(conn):
    # PN: BMU "PEHE-1" planned at 100 MW for 12:00-12:30, ramp to 120 by 13:00.
    upsert(
        conn,
        "pn",
        [
            {
                "national_grid_bm_unit": "PEHE-1",
                "bm_unit": "T_PEHE-1",
                "settlement_date": None,
                "settlement_period": None,
                "time_from": datetime(2026, 5, 1, 12, 0),
                "time_to": datetime(2026, 5, 1, 12, 30),
                "level_from": 100.0,
                "level_to": 100.0,
            },
            {
                "national_grid_bm_unit": "PEHE-1",
                "bm_unit": "T_PEHE-1",
                "settlement_date": None,
                "settlement_period": None,
                "time_from": datetime(2026, 5, 1, 12, 30),
                "time_to": datetime(2026, 5, 1, 13, 0),
                "level_from": 100.0,
                "level_to": 120.0,
            },
        ],
        ["national_grid_bm_unit", "time_from"],
    )
    # BOALF: a bid acceptance dropping it to 60 MW at 12:10-12:20 (so_flag = True).
    upsert(
        conn,
        "boalf",
        [
            {
                "acceptance_id": 1,
                "bm_unit": "T_PEHE-1",
                "acceptance_time": datetime(2026, 5, 1, 12, 5),
                "time_from": datetime(2026, 5, 1, 12, 10),
                "time_to": datetime(2026, 5, 1, 12, 20),
                "level_from": 60.0,
                "level_to": 60.0,
                "deemed_bo_flag": False,
                "so_flag": True,
            }
        ],
        ["acceptance_id", "time_from"],
    )


def test_bmu_dispatch_curtailment_and_turnup(db):
    _seed(db)
    df = bmu_dispatch(
        db,
        ["PEHE-1"],
        datetime(2026, 5, 1, 12, 0),
        datetime(2026, 5, 1, 13, 0),
        freq="1min",
    )
    assert len(df) == 60

    # Outside acceptance: boa_level_mw == pn_mw, no curtailment.
    pre = df[df["ts"] == datetime(2026, 5, 1, 12, 5)].iloc[0]
    assert pre["pn_mw"] == 100.0
    assert pre["boa_level_mw"] == 100.0
    assert pre["boa_curtailment_mw"] == 0.0
    assert pre["so_curtailment_mw"] == 0.0
    assert pre["so_turnup_mw"] == 0.0

    # During acceptance: 100 → 60 = 40 MW of SO curtailment.
    mid = df[df["ts"] == datetime(2026, 5, 1, 12, 15)].iloc[0]
    assert mid["pn_mw"] == 100.0
    assert mid["boa_level_mw"] == 60.0
    assert mid["boa_curtailment_mw"] == 40.0
    assert mid["so_curtailment_mw"] == 40.0
    assert mid["so_turnup_mw"] == 0.0
