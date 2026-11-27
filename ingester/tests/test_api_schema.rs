//! DB-backed tests for the web UI's `api` schema
//! (`src/gb_grid/migrations/post-apply.api.sql`).
//!
//! Every `api.*` function is called as `web_anon`, the role PostgREST runs
//! requests as, so these also check that the grants are enough to read through
//! the hypertables and continuous aggregates. Where a function ports a Grafana
//! panel query, its result is compared with that original SQL on the same data.
//!
//! Skips when `GB_GRID_DATABASE_URL` is unset (see `tests/common/mod.rs`).

mod common;

use std::sync::{Mutex, MutexGuard};

use tokio_postgres::Client;

static DB_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    DB_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn assert_close(a: f64, b: f64, label: &str) {
    assert!(
        (a - b).abs() <= 1e-9 * (1.0 + a.abs().max(b.abs())),
        "{label}: expected {b}, got {a}"
    );
}

/// A two-BMU wind station `ZZTST` (a code no real BMU uses) with two days of
/// dispatch, daily totals, MEL, B1610, plus fuelinst and constraint rows.
async fn seed(conn: &Client) {
    conn.batch_execute(
        r#"
        DELETE FROM bmu WHERE station = 'ZZTST';
        INSERT INTO bmu (ngc_bm_unit, bm_unit, bmrs_fuel_type, latitude, longitude, bmu_name) VALUES
          ('ZZTST-1', 'T_ZZTST-1', 'WIND', 57.0, -2.0, 'Test Wind Farm 1'),
          ('ZZTST-2', 'T_ZZTST-2', 'WIND', 57.2, -2.2, 'Test Wind Farm 2');

        TRUNCATE bmu_dispatch, bmu_dispatch_daily, mels, b1610, fuelinst, constraints;

        INSERT INTO bmu_dispatch (bmu, ts, pn_mw, boa_level_mw, mel_mw, so_turnup_mw,
                                  boa_curtailment_mw, so_curtailment_mw)
        SELECT b.bmu, t,
               100 + b.k * 10 + extract(hour FROM t),
               80 + b.k, 150, 0, 20 + b.k, 5
        FROM (VALUES ('ZZTST-1', 0), ('ZZTST-2', 1)) AS b (bmu, k),
             generate_series('2026-05-13 00:00'::timestamp, '2026-05-14 23:55', '5 min') AS t;

        INSERT INTO bmu_dispatch_daily (bmu, date, pn_mwh, boa_dispatched_mwh, so_turnup_mwh,
                                        boa_curtailment_mwh, so_curtailment_mwh, b1610_mwh) VALUES
          ('ZZTST-1', '2026-05-13', 2400, 1900, 0, 480, 120, 2000),
          ('ZZTST-2', '2026-05-13', 2600, 1950, 10, 500, 120, NULL),
          ('ZZTST-1', '2026-05-14', 2500, 1800, 0, 400, 120, 2100),
          ('ZZTST-2', '2026-05-14', 2700, 2000, 5, 420, 120, 2200);

        INSERT INTO mels (national_grid_bm_unit, time_from, time_to, level_from, level_to,
                          notification_sequence) VALUES
          ('ZZTST-1', '2026-05-13 00:00', '2026-05-15 00:00', 150, 150, 1),
          ('ZZTST-2', '2026-05-13 00:00', '2026-05-15 00:00', 140, 160, 1),
          ('ZZTST-1', '2026-03-28 00:00', '2026-03-31 00:00', 100, 100, 1);

        -- Half-hourly metered output for 13 May (BST) and 29 March 2026, the
        -- spring clock change (46 settlement periods).
        INSERT INTO b1610 (settlement_date, settlement_period, bm_unit, ngc_bm_unit, quantity_mwh)
        SELECT '2026-05-13'::date, sp, 'T_' || u, u, 25 + sp
        FROM generate_series(1, 48) sp, unnest(ARRAY['ZZTST-1', 'ZZTST-2']) u;
        INSERT INTO b1610 (settlement_date, settlement_period, bm_unit, ngc_bm_unit, quantity_mwh)
        SELECT '2026-03-29'::date, sp, 'T_ZZTST-1', 'ZZTST-1', sp
        FROM generate_series(1, 46) sp;

        INSERT INTO fuelinst (publish_time, fuel_type, generation_mw)
        SELECT t, f, CASE f WHEN 'WIND' THEN 5000 WHEN 'CCGT' THEN 3000 ELSE -500 END
        FROM generate_series('2026-05-13 00:00'::timestamp, '2026-05-13 23:55', '5 min') t,
             unnest(ARRAY['WIND', 'CCGT', 'INTFR']) f;

        INSERT INTO constraints (constraint_group, ts, limit_mw, flow_mw)
        SELECT 'SCOTEX', t, 3000, 1000 + extract(hour FROM t) * 50
        FROM generate_series('2026-05-12 00:00'::timestamp, '2026-05-14 23:30', '30 min') t;
        "#,
    )
    .await
    .expect("seed");

    // Continuous aggregates only show refreshed data (materialized-only).
    for (cagg, from, to) in [
        ("bmu_dispatch_daily_mw", "2026-05-01", "2026-06-01"),
        ("bmu_dispatch_hourly", "2026-05-01", "2026-06-01"),
        ("b1610_hh_cagg", "2026-03-01", "2026-06-01"),
        ("b1610_daily_cagg", "2026-03-01", "2026-06-01"),
    ] {
        conn.execute(
            &format!("CALL refresh_continuous_aggregate('{cagg}', '{from}', '{to}')"),
            &[],
        )
        .await
        .unwrap_or_else(|e| panic!("refresh {cagg}: {e}"));
    }
}

async fn as_web_anon(conn: &Client) {
    conn.batch_execute("SET ROLE web_anon").await.expect("SET ROLE web_anon");
}

async fn reset_role(conn: &Client) {
    conn.batch_execute("RESET ROLE").await.expect("RESET ROLE");
}

async fn scalar(conn: &Client, sql: &str) -> Option<f64> {
    conn.query_one(sql, &[])
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
        .get(0)
}

#[tokio::test]
async fn test_api_schema() {
    let _g = lock();
    let Some(conn) = common::connect().await else {
        eprintln!("SKIP: no database");
        return;
    };
    seed(&conn).await;
    as_web_anon(&conn).await;

    // --- stations: name, location, fuel, BMU count, has_dispatch ---------------
    let row = conn
        .query_one(
            "SELECT name, latitude, fuel_type, bmus, has_dispatch FROM api.stations() \
             WHERE station = 'ZZTST'",
            &[],
        )
        .await
        .expect("api.stations");
    assert_eq!(row.get::<_, Option<String>>(0).as_deref(), Some("Test Wind Farm"));
    assert_close(row.get::<_, f64>(1), 57.1, "station latitude");
    assert_eq!(row.get::<_, String>(2), "WIND");
    assert_eq!(row.get::<_, i32>(3), 2);
    assert!(row.get::<_, bool>(4), "has_dispatch");

    let fuels = conn.query("SELECT fuel_type FROM api.fuel_types()", &[]).await.unwrap();
    assert!(fuels.iter().any(|r| r.get::<_, String>(0) == "WIND"));

    // --- dispatch series: the api wrapper equals the Grafana panel query -------
    for step in ["5 minutes", "1 hour", "1 day"] {
        let api = scalar(
            &conn,
            &format!(
                "SELECT sum(pn_mw + boa_level_mw + mel_mw + boa_curtailment_mw) FROM api.dispatch_series(\
                 'ZZTST', '2026-05-13T00:00Z', '2026-05-15T00:00Z', '{step}')"
            ),
        )
        .await
        .unwrap();
        let grafana = scalar(
            &conn,
            &format!(
                "SELECT sum(pn_mw + boa_level_mw + mel_mw + boa_curtailment_mw) FROM dispatch_series(\
                 'ZZTST', '2026-05-13 00:00', '2026-05-15 00:00', '{step}')"
            ),
        )
        .await
        .unwrap();
        assert_close(api, grafana, &format!("dispatch_series at {step}"));
    }
    let first: chrono::DateTime<chrono::Utc> = conn
        .query_one(
            "SELECT min(ts) FROM api.dispatch_series('ZZTST', '2026-05-13T01:00+01:00', \
             '2026-05-13T03:00Z', '5 minutes')",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(first.to_rfc3339(), "2026-05-13T00:00:00+00:00", "timestamptz in, UTC out");

    let api_b1610 = scalar(
        &conn,
        "SELECT sum(b1610_mw) FROM api.b1610_series('ZZTST', '2026-05-12T23:00Z', \
         '2026-05-13T23:00Z', '30 minutes')",
    )
    .await
    .unwrap();
    let grafana_b1610 = scalar(
        &conn,
        "SELECT sum(b1610_mw) FROM b1610_series('ZZTST', '2026-05-12 23:00', \
         '2026-05-13 23:00', '30 minutes')",
    )
    .await
    .unwrap();
    assert_close(api_b1610, grafana_b1610, "b1610_series");

    // --- stat tiles ---------------------------------------------------------------
    let t = conn
        .query_one(
            "SELECT pn_mwh, b1610_mwh, turnup_mwh, turndown_mwh, pn_minus_b1610_mwh \
             FROM api.dispatch_totals('ZZTST', '2026-05-13T00:00Z', '2026-05-14T12:00Z')",
            &[],
        )
        .await
        .unwrap();
    // Grafana: SUM over days whose 00:00 UTC is in the window (13 and 14 May).
    assert_close(t.get::<_, f64>(0), 10200.0, "pn_mwh");
    assert_close(t.get::<_, f64>(1), 6300.0, "b1610_mwh");
    assert_close(t.get::<_, f64>(2), 15.0, "turnup_mwh");
    assert_close(t.get::<_, f64>(3), 1800.0, "turndown_mwh");
    assert_close(t.get::<_, f64>(4), 3900.0, "pn_minus_b1610_mwh");

    // --- annual calendar, MWh and % of MEL capacity ----------------------------
    let rows = conn
        .query(
            "SELECT day::text, pn, b1610 FROM api.daily_calendar('2026-05-01T00:00Z', \
             '2026-06-01T00:00Z', '{WIND}', '{ZZTST}', false) ORDER BY day",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].get::<_, String>(0), "2026-05-13");
    assert_close(rows[0].get::<_, f64>(1), 5000.0, "calendar pn MWh");
    assert_close(rows[0].get::<_, f64>(2), 2000.0, "calendar b1610 MWh");

    let api_pct = scalar(
        &conn,
        "SELECT pn FROM api.daily_calendar('2026-05-01T00:00Z', '2026-06-01T00:00Z', \
         '{WIND}', '{ZZTST}', true) WHERE day = '2026-05-14'",
    )
    .await
    .unwrap();
    // The Grafana `_pct` panel query for the same selection.
    let grafana_pct = scalar(
        &conn,
        "WITH cap AS (SELECT COALESCE(SUM(m.cap),0) AS cap FROM (SELECT national_grid_bm_unit AS ngc, \
         MAX(GREATEST(level_from, level_to)) AS cap FROM mels WHERE time_from >= '2026-05-01' AND \
         time_from < '2026-06-01' GROUP BY 1) m JOIN bmu b ON b.ngc_bm_unit = m.ngc WHERE \
         b.station IN ('ZZTST') AND COALESCE(b.bmrs_fuel_type, 'UNKNOWN') IN ('WIND')) \
         SELECT 100.0 * SUM(d.pn_mwh) / NULLIF((SELECT cap FROM cap) * 24.0, 0) FROM \
         bmu_dispatch_daily d JOIN bmu b ON b.ngc_bm_unit = d.bmu WHERE b.station IN ('ZZTST') \
         AND COALESCE(b.bmrs_fuel_type, 'UNKNOWN') IN ('WIND') AND d.date = '2026-05-14'",
    )
    .await
    .unwrap();
    assert_close(api_pct, grafana_pct, "calendar % of capacity");
    assert_close(api_pct, 100.0 * 5200.0 / (310.0 * 24.0), "calendar % value");

    // --- fuel mix ---------------------------------------------------------------
    let mix = conn
        .query(
            "SELECT fuel_type, cardinality(ts), mw[1] FROM api.fuel_mix('2026-05-13T00:00Z', \
             '2026-05-14T00:00Z', '1 hour') ORDER BY fuel_type",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(mix.len(), 3);
    assert_eq!(mix[0].get::<_, String>(0), "CCGT");
    assert_eq!(mix[0].get::<_, i32>(1), 24);
    assert_close(mix[1].get::<_, f64>(2), -500.0, "INTFR hourly mean");

    // --- station grid -------------------------------------------------------------
    // 13 May is BST: settlement period 1 starts at 23:00 UTC on the 12th.
    let g = conn
        .query_one(
            "SELECT rank, ts[1], value[1] FROM api.station_grid('WIND', '2026-05-12T23:00Z', \
             '2026-05-13T23:00Z', false) WHERE station = 'ZZTST'",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(g.get::<_, i64>(1), 1_778_626_800_000, "SP1 of 13 May = 2026-05-12T23:00Z");
    assert_close(g.get::<_, f64>(2), 2.0 * (26.0 + 26.0), "net MW, both BMUs");
    let pct = scalar(
        &conn,
        "SELECT value[1] FROM api.station_grid('WIND', '2026-05-12T23:00Z', '2026-05-13T23:00Z', true) \
         WHERE station = 'ZZTST'",
    )
    .await
    .unwrap();
    assert_close(pct, 100.0 * 104.0 / 310.0, "load factor");

    // Spring-forward day: SP3 is 01:00 UTC (02:00 BST), and the day has 46
    // distinct half-hours.
    let spring = conn
        .query_one(
            "SELECT ts[3], cardinality(ts) FROM api.station_grid('WIND', '2026-03-29T00:00Z', \
             '2026-03-30T00:00Z', false) WHERE station = 'ZZTST'",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(spring.get::<_, i64>(0), 1_774_746_000_000, "SP3 on 29 Mar = 01:00Z");
    assert_eq!(spring.get::<_, i32>(1), 46);

    // --- constraints ------------------------------------------------------------
    let groups = conn
        .query(
            "SELECT label, constraint_group FROM api.constraint_groups('2026-05-13T00:00Z', \
             '2026-05-14T00:00Z')",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].get::<_, String>(0), "B6 — SCOTEX");

    let c = conn
        .query_one(
            "SELECT cardinality(ts), flow_mw[48], avg_24h_mw[48], daily_max_mw[1], \
             daily_min_mw[1], limit_mw[1] FROM api.constraint_series('{SCOTEX}', \
             '2026-05-13T00:00Z', '2026-05-14T00:00Z')",
            &[],
        )
        .await
        .unwrap();
    // BETWEEN is inclusive at both ends, as in the Grafana panel: 49 rows.
    assert_eq!(c.get::<_, i32>(0), 49);
    assert_close(c.get::<_, f64>(1), 1000.0 + 23.0 * 50.0, "flow at 23:30");
    assert_close(c.get::<_, f64>(2), 1000.0 + 50.0 * 11.5, "rolling 24h mean");
    assert_close(c.get::<_, f64>(3), 1000.0 + 23.0 * 50.0, "daily max");
    assert_close(c.get::<_, f64>(4), 1000.0, "daily min");
    assert_close(c.get::<_, f64>(5), 3000.0, "limit");

    // --- web_anon is read-only ----------------------------------------------------
    let err = conn
        .execute("DELETE FROM bmu_dispatch_daily", &[])
        .await
        .expect_err("web_anon must not be able to write");
    assert_eq!(
        err.code(),
        Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE),
        "{err:?}"
    );

    reset_role(&conn).await;
}

/// Re-running `gb-grid migrate` with nothing pending still re-applies the hook,
/// which must be idempotent and leave the functions callable.
#[tokio::test]
async fn test_api_schema_reapplies_idempotently() {
    let _g = lock();
    let Some(conn) = common::connect().await else {
        eprintln!("SKIP: no database");
        return;
    };
    conn.batch_execute("DROP FUNCTION api.fuel_types()").await.unwrap();
    common::migrate_again().expect("second migrate");
    common::migrate_again().expect("third migrate");
    as_web_anon(&conn).await;
    let n = conn.query("SELECT * FROM api.fuel_types()", &[]).await.unwrap().len();
    assert!(n > 0, "api.fuel_types() recreated by the post-apply hook");
    reset_role(&conn).await;
}
