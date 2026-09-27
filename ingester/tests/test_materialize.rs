//! DB-backed tests for `materialize_dispatch` / `materialize_dispatch_daily`.
//!
//! Skips (prints SKIP) when `GB_GRID_DATABASE_URL` is unset or the scratch DB
//! can't be bootstrapped (see `tests/common/mod.rs`).

mod common;

use std::sync::{Mutex, MutexGuard};

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use tokio_postgres::Client;

use gb_grid_ingest::materialize::{materialize_dispatch, materialize_dispatch_daily};

// The scratch DB is shared per process and `#[tokio::test]` bodies run in
// parallel; serialize the DB tests and truncate before each so they can't leak
// rows or collide on `refresh_continuous_aggregate` (which refuses concurrent
// refreshes of the same cagg).
static DB_LOCK: Mutex<()> = Mutex::new(());

const TABLES: &[&str] = &["pn", "boalf", "mels", "bmu_dispatch", "bmu_dispatch_daily"];

fn lock() -> MutexGuard<'static, ()> {
    DB_LOCK.lock().unwrap()
}

fn dt(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> NaiveDateTime {
    NaiveDateTime::new(
        NaiveDate::from_ymd_opt(y, mo, d).unwrap(),
        NaiveTime::from_hms_opt(h, mi, s).unwrap(),
    )
}

fn d(y: i32, mo: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, mo, day).unwrap()
}

fn assert_close(a: f64, b: f64, label: &str) {
    if a.is_nan() && b.is_nan() {
        return;
    }
    assert!(
        (a - b).abs() <= 1e-9 * (1.0 + a.abs().max(b.abs())),
        "{label}: expected {b}, got {a}"
    );
}

fn chk(actual: Option<f64>, expected: Option<f64>, label: &str) {
    match (actual, expected) {
        (Some(a), Some(e)) => assert_close(a, e, label),
        (None, None) => {}
        (a, e) => panic!("{label}: expected {e:?}, got {a:?}"),
    }
}

/// One BMU over 12:00-13:00 UTC. PN is 100 MW then ramps to 120; MEL caps at
/// 110 then 90; BOA has an SO turnup (110), an SO curtailment (60), a
/// non-SO-flagged curtailment (80), and two overlapping acceptances at
/// 12:30-12:40 where the NULL-acceptance_time one (70) must win.
async fn seed(conn: &Client) {
    for (tf, tt, lf, lt) in [
        (dt(2026, 5, 1, 12, 0, 0), dt(2026, 5, 1, 12, 30, 0), 100.0, 100.0),
        (dt(2026, 5, 1, 12, 30, 0), dt(2026, 5, 1, 13, 0, 0), 100.0, 120.0),
    ] {
        conn.execute(
            "INSERT INTO pn \
             (national_grid_bm_unit, bm_unit, settlement_date, settlement_period, \
              time_from, time_to, level_from, level_to) \
             VALUES ($1, $2, NULL, NULL, $3, $4, $5, $6)",
            &[&"PEHE-1", &"T_PEHE-1", &tf, &tt, &lf, &lt],
        )
        .await
        .unwrap();
    }

    // (acceptance_id, acceptance_time, time_from, time_to, level, so_flag)
    let boas: Vec<(i64, Option<NaiveDateTime>, NaiveDateTime, NaiveDateTime, f64, Option<bool>)> = vec![
        (1, Some(dt(2026, 5, 1, 11, 55, 0)), dt(2026, 5, 1, 12, 0, 0), dt(2026, 5, 1, 12, 5, 0), 110.0, Some(true)),
        (2, Some(dt(2026, 5, 1, 12, 5, 0)), dt(2026, 5, 1, 12, 10, 0), dt(2026, 5, 1, 12, 20, 0), 60.0, Some(true)),
        (3, Some(dt(2026, 5, 1, 12, 10, 0)), dt(2026, 5, 1, 12, 20, 0), dt(2026, 5, 1, 12, 25, 0), 80.0, None),
        (4, None, dt(2026, 5, 1, 12, 30, 0), dt(2026, 5, 1, 12, 40, 0), 70.0, Some(true)),
        (5, Some(dt(2026, 5, 1, 12, 20, 0)), dt(2026, 5, 1, 12, 30, 0), dt(2026, 5, 1, 12, 40, 0), 95.0, Some(true)),
    ];
    for (id, at, tf, tt, lvl, so) in boas {
        conn.execute(
            "INSERT INTO boalf \
             (acceptance_id, bm_unit, ngc_bm_unit, acceptance_time, time_from, time_to, \
              level_from, level_to, deemed_bo_flag, so_flag) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $7, $8, $9)",
            &[&id, &"T_PEHE-1", &"PEHE-1", &at, &tf, &tt, &lvl, &false, &so],
        )
        .await
        .unwrap();
    }

    for (tf, tt, lvl, seq) in [
        (dt(2026, 5, 1, 12, 0, 0), dt(2026, 5, 1, 12, 30, 0), 110.0, 1_i64),
        (dt(2026, 5, 1, 12, 30, 0), dt(2026, 5, 1, 13, 0, 0), 90.0, 2_i64),
    ] {
        conn.execute(
            "INSERT INTO mels \
             (national_grid_bm_unit, bm_unit, settlement_date, settlement_period, \
              time_from, time_to, level_from, level_to, notification_time, notification_sequence) \
             VALUES ($1, $2, NULL, NULL, $3, $4, $5, $5, NULL, $6)",
            &[&"PEHE-1", &"T_PEHE-1", &tf, &tt, &lvl, &seq],
        )
        .await
        .unwrap();
    }
}

async fn dispatch_row(
    conn: &Client,
    ts: NaiveDateTime,
) -> (Option<f64>, Option<f64>, Option<f64>, Option<f64>, Option<f64>, Option<f64>) {
    let row = conn
        .query_one(
            "SELECT pn_mw, boa_level_mw, mel_mw, so_turnup_mw, boa_curtailment_mw, so_curtailment_mw \
             FROM bmu_dispatch WHERE bmu = $1 AND ts = $2",
            &[&"PEHE-1", &ts],
        )
        .await
        .unwrap();
    (row.get(0), row.get(1), row.get(2), row.get(3), row.get(4), row.get(5))
}

async fn assert_dispatch(
    conn: &Client,
    min: u32,
    pn: f64,
    boa: f64,
    mel: f64,
    turnup: f64,
    curt: f64,
    so_curt: f64,
) {
    let (a_pn, a_boa, a_mel, a_turnup, a_curt, a_so_curt) =
        dispatch_row(conn, dt(2026, 5, 1, 12, min, 0)).await;
    chk(a_pn, Some(pn), &format!("12:{min:02} pn_mw"));
    chk(a_boa, Some(boa), &format!("12:{min:02} boa_level_mw"));
    chk(a_mel, Some(mel), &format!("12:{min:02} mel_mw"));
    chk(a_turnup, Some(turnup), &format!("12:{min:02} so_turnup_mw"));
    chk(a_curt, Some(curt), &format!("12:{min:02} boa_curtailment_mw"));
    chk(a_so_curt, Some(so_curt), &format!("12:{min:02} so_curtailment_mw"));
}

#[tokio::test]
async fn test_materialize_dispatch_and_daily() {
    let Some(cfg) = common::config() else {
        eprintln!("SKIP");
        return;
    };
    let Some(conn) = common::connect().await else {
        eprintln!("SKIP");
        return;
    };
    let _guard = lock();
    common::truncate(&conn, TABLES).await;

    seed(&conn).await;

    let start = dt(2026, 5, 1, 12, 0, 0);
    let end = dt(2026, 5, 1, 13, 0, 0);
    let n = materialize_dispatch(&cfg, start, end, Some(&["PEHE-1".to_string()]), Some(1), 5)
        .await
        .unwrap();
    assert_eq!(n, 12, "12 grid points 12:00..12:55, all with PN");

    // 12:00 — SO turnup acceptance 110 vs PN 100.
    assert_dispatch(&conn, 0, 100.0, 110.0, 110.0, 10.0, 0.0, 0.0).await;
    // 12:05 — outside any acceptance: boa == pn.
    assert_dispatch(&conn, 5, 100.0, 100.0, 110.0, 0.0, 0.0, 0.0).await;
    // 12:10 / 12:15 — SO curtailment to 60.
    assert_dispatch(&conn, 10, 100.0, 60.0, 110.0, 0.0, 40.0, 40.0).await;
    assert_dispatch(&conn, 15, 100.0, 60.0, 110.0, 0.0, 40.0, 40.0).await;
    // 12:20 — 80 MW acceptance with NULL so_flag: boa curtailment but NOT so.
    assert_dispatch(&conn, 20, 100.0, 80.0, 110.0, 0.0, 20.0, 0.0).await;
    // 12:25 — outside acceptance again.
    assert_dispatch(&conn, 25, 100.0, 100.0, 110.0, 0.0, 0.0, 0.0).await;
    // 12:30 / 12:35 — MEL caps PN at 90; NULL-acceptance_time 70 wins the overlap.
    assert_dispatch(&conn, 30, 90.0, 70.0, 90.0, 0.0, 20.0, 20.0).await;
    assert_dispatch(&conn, 35, 90.0, 70.0, 90.0, 0.0, 20.0, 20.0).await;
    // 12:40..12:55 — no acceptance; PN ramps but MEL caps it at 90.
    assert_dispatch(&conn, 40, 90.0, 90.0, 90.0, 0.0, 0.0, 0.0).await;
    assert_dispatch(&conn, 45, 90.0, 90.0, 90.0, 0.0, 0.0, 0.0).await;
    assert_dispatch(&conn, 50, 90.0, 90.0, 90.0, 0.0, 0.0, 0.0).await;
    assert_dispatch(&conn, 55, 90.0, 90.0, 90.0, 0.0, 0.0, 0.0).await;

    // Daily rollup. 5-min samples → MWh = MW * 5/60.
    let date = d(2026, 5, 1);
    let n_daily = materialize_dispatch_daily(&cfg, date, date).await.unwrap();
    assert_eq!(n_daily, 1);

    let row = conn
        .query_one(
            "SELECT pn_mwh, boa_dispatched_mwh, so_turnup_mwh, boa_curtailment_mwh, \
                    so_curtailment_mwh, b1610_mwh \
             FROM bmu_dispatch_daily WHERE bmu = $1 AND date = $2",
            &[&"PEHE-1", &date],
        )
        .await
        .unwrap();
    // pn_mw sums: 6*100 + 6*90 = 1140; 1140 * 5/60 = 95.
    chk(row.get::<_, Option<f64>>(0), Some(95.0), "pn_mwh");
    // boa_level_mw sums: 110+100+60+60+80+100+70+70+90+90+90+90 = 1010.
    chk(row.get::<_, Option<f64>>(1), Some(1010.0 * 5.0 / 60.0), "boa_dispatched_mwh");
    // so_turnup sums: 10.
    chk(row.get::<_, Option<f64>>(2), Some(10.0 * 5.0 / 60.0), "so_turnup_mwh");
    // boa_curtailment sums: 40+40+20+20+20 = 140.
    chk(row.get::<_, Option<f64>>(3), Some(140.0 * 5.0 / 60.0), "boa_curtailment_mwh");
    // so_curtailment sums: 40+40+20+20 = 120.
    chk(row.get::<_, Option<f64>>(4), Some(120.0 * 5.0 / 60.0), "so_curtailment_mwh");
    // No B1610 rows seeded → NULL.
    chk(row.get::<_, Option<f64>>(5), None, "b1610_mwh");
}

#[tokio::test]
async fn test_materialize_dispatch_skips_rows_with_no_pn_or_boa() {
    let Some(cfg) = common::config() else {
        eprintln!("SKIP");
        return;
    };
    let Some(conn) = common::connect().await else {
        eprintln!("SKIP");
        return;
    };
    let _guard = lock();
    common::truncate(&conn, TABLES).await;

    // A BOALF-only unit with a BOA outside the window: neither PN nor BOA cover
    // any grid point → every row is dropped, nothing written.
    conn.execute(
        "INSERT INTO pn \
         (national_grid_bm_unit, bm_unit, settlement_date, settlement_period, \
          time_from, time_to, level_from, level_to) \
         VALUES ($1, $2, NULL, NULL, $3, $4, $5, $6)",
        &[&"PNONLY-1", &"T_PNONLY-1", &dt(2026, 5, 1, 14, 0, 0), &dt(2026, 5, 1, 15, 0, 0), &50.0, &50.0],
    )
    .await
    .unwrap();

    let start = dt(2026, 5, 1, 12, 0, 0);
    let end = dt(2026, 5, 1, 13, 0, 0);
    let n = materialize_dispatch(&cfg, start, end, Some(&["PNONLY-1".to_string()]), Some(1), 5)
        .await
        .unwrap();
    assert_eq!(n, 0, "no grid point has pn or boa level");
    let count: i64 = conn
        .query_one("SELECT count(*) FROM bmu_dispatch WHERE bmu = $1", &[&"PNONLY-1"])
        .await
        .unwrap()
        .get(0);
    assert_eq!(count, 0);
}

#[tokio::test]
async fn test_materialize_dispatch_empty_bmus_falls_back_to_active() {
    let Some(cfg) = common::config() else {
        eprintln!("SKIP");
        return;
    };
    let Some(conn) = common::connect().await else {
        eprintln!("SKIP");
        return;
    };
    let _guard = lock();
    common::truncate(&conn, TABLES).await;

    conn.execute(
        "INSERT INTO pn \
         (national_grid_bm_unit, bm_unit, settlement_date, settlement_period, \
          time_from, time_to, level_from, level_to) \
         VALUES ($1, $2, NULL, NULL, $3, $4, $5, $6)",
        &[&"ACTIVE-1", &"T_ACTIVE-1", &dt(2026, 5, 1, 12, 0, 0), &dt(2026, 5, 1, 13, 0, 0), &10.0, &10.0],
    )
    .await
    .unwrap();

    let start = dt(2026, 5, 1, 12, 0, 0);
    let end = dt(2026, 5, 1, 13, 0, 0);
    // Empty slice → falls back to _active_bmus (the unit we just inserted).
    let n = materialize_dispatch(&cfg, start, end, Some(&[]), Some(1), 5)
        .await
        .unwrap();
    assert_eq!(n, 12);
    let count: i64 = conn
        .query_one("SELECT count(*) FROM bmu_dispatch WHERE bmu = $1", &[&"ACTIVE-1"])
        .await
        .unwrap()
        .get(0);
    assert_eq!(count, 12);
}