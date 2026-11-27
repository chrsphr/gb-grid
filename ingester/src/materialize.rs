//! Per-BMU dispatch materialization: combine PN + BOALF (+ MEL) segments onto a
//! fixed grid, then upsert into `bmu_dispatch` and refresh its caggs.
//!
//! Port of `materialize.py` + `analytics.bmu_dispatch`.
//!
//! Definitions (must match `analytics.py` / `materialize.py` exactly):
//! - `pn_mw`: PN level clipped to MEL where MEL is known (MEL is a physical cap).
//! - `boa_level_mw`: winning BOA acceptance (later `acceptance_time` wins on
//!   overlap), clipped to MEL; falls back to `pn_mw` where no acceptance active.
//! - `so_turnup_mw` = max(boa_level - pn, 0)
//! - `boa_curtailment_mw` = max(pn - boa_level, 0)
//! - `so_curtailment_mw` = max(pn - boa_so, 0) with NaN → 0 (only this column
//!   gets the zero-fill, matching `.clip(lower=0).fillna(0)`).
//!
//! Row output rules (must match `materialize._rows_for_units`):
//! - Drop rows where BOTH `pn_mw` and `boa_level_mw` are NaN.
//! - pn/boa/mel NaN → NULL; the three derived columns are raw f64 (NaN stays
//!   NaN in the table, matching psycopg).
//! - Columns: `bmu, ts, pn_mw, boa_level_mw, mel_mw, so_turnup_mw, boa_curtailment_mw, so_curtailment_mw`.
//! - `ts` is floored to the freq boundary; grid = `date_range(start, end, freq, inclusive="left")`.
//!
//! Concurrency: split BMUs round-robin across `workers` (default ncpu-1) worker
//! tasks, each with its own `db::connect` connection for fetch + upsert (writes
//! may land out of order — per-chunk keys are disjoint). `freq_minutes` is 5 by
//! default (1 supported).
//!
//! After the upserts, refresh `DISPATCH_CAGGS` over the whole-day window
//! `[start.date 00:00, end.date+1d 00:00)` on a fresh connection.
//!
//! `materialize_dispatch_daily` is the SQL port of `materialize.py`:
//! FULL OUTER JOIN of `bmu_dispatch_daily_cagg` and `b1610_daily_cagg` into
//! `bmu_dispatch_daily` with `ON CONFLICT (bmu, date) DO UPDATE`, MW→MWh via
//! `_DAILY_INTERVAL_HOURS = 5/60`.

use anyhow::{Context, Result};
use chrono::{Days, NaiveDate, NaiveDateTime, NaiveTime};
use tokio_postgres::Client;

use crate::config::{cli_date, Config};
use crate::db::{self, Row, Value};
use crate::interp::{interp_segments, Segment, SortKey};

pub const TABLE: &str = "bmu_dispatch";
pub const CONFLICT_COLS: [&str; 2] = ["bmu", "ts"];

/// Continuous aggregates built on bmu_dispatch, refreshed after every
/// materialize (their policies only reach back ~30 days).
pub const DISPATCH_CAGGS: [&str; 3] = [
    "bmu_dispatch_hourly",
    "bmu_dispatch_daily_mw",
    "bmu_dispatch_daily_cagg",
];

pub const DAILY_INTERVAL_HOURS: f64 = 5.0 / 60.0;

/// Upsert column order for `bmu_dispatch` (must match `materialize._rows_for_units`).
const DISPATCH_COLS: [&str; 8] = [
    "bmu",
    "ts",
    "pn_mw",
    "boa_level_mw",
    "mel_mw",
    "so_turnup_mw",
    "boa_curtailment_mw",
    "so_curtailment_mw",
];

/// One BOALF row as needed for the per-unit interpolation. Fetched per unit, so
/// the `bm_unit` / `ngc_bm_unit` / `acceptance_id` columns aren't needed here.
#[derive(Clone)]
struct BoalfSeg {
    acceptance_time: Option<NaiveDateTime>,
    time_from: NaiveDateTime,
    time_to: NaiveDateTime,
    level_from: f64,
    level_to: f64,
    so_flag: Option<bool>,
}

pub async fn materialize_dispatch(
    cfg: &Config,
    start: NaiveDateTime,
    end: NaiveDateTime,
    bmus: Option<&[String]>,
    workers: Option<usize>,
    freq_minutes: i64,
) -> Result<usize> {
    let units: Vec<String> = match bmus {
        // Python: `units = bmus or _active_bmus(...)` — an empty list falls back.
        Some(u) if !u.is_empty() => u.to_vec(),
        _ => {
            let conn = db::connect(cfg).await?;
            active_bmus(&conn, start, end).await?
        }
    };
    if units.is_empty() {
        crate::log_info!("materialize_dispatch_no_bmus", "start", start, "end", end);
        return Ok(0);
    }

    let n_workers = workers
        .unwrap_or(
            (std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(2))
            .saturating_sub(1)
            .max(1),
        )
        .min(units.len())
        .max(1);

    // Round-robin chunks so each worker sees a mix of heavy/light BMUs.
    let chunks: Vec<Vec<String>> = (0..n_workers)
        .map(|i| units.iter().skip(i).step_by(n_workers).cloned().collect())
        .collect();

    let cols: Vec<String> = DISPATCH_COLS.iter().map(|s| s.to_string()).collect();
    let mut handles = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        let cfg = cfg.clone();
        let cols = cols.clone();
        handles.push(tokio::spawn(async move {
            let conn = db::connect(&cfg).await.context("dispatch worker connect")?;
            let rows = dispatch_rows(&conn, &chunk, start, end, freq_minutes).await?;
            let n = db::upsert(&conn, TABLE, &cols, &rows, &CONFLICT_COLS)
                .await
                .context("dispatch worker upsert")?;
            anyhow::Result::<usize>::Ok(n)
        }));
    }
    let results = futures::future::join_all(handles).await;
    let mut total = 0usize;
    for r in results {
        total += r.context("dispatch worker task")??;
    }

    // Fold the freshly-written rows into the caggs the dashboards read. Snap to
    // whole UTC days (refresh_continuous_aggregate errors on sub-bucket windows).
    let conn = db::connect(cfg).await?;
    let ref_start = start.date().and_hms_opt(0, 0, 0).unwrap();
    let ref_end = (end.date() + Days::new(1)).and_hms_opt(0, 0, 0).unwrap();
    db::refresh_caggs(&conn, &DISPATCH_CAGGS, ref_start, ref_end)
        .await
        .context("refresh dispatch caggs")?;

    crate::log_info!(
        "materialize_dispatch_wrote",
        "rows", total,
        "units", units.len(),
        "workers", n_workers,
        "start", start,
        "end", end
    );
    Ok(total)
}

/// BMUs with PN data in the window (port of `materialize._active_bmus`).
async fn active_bmus(conn: &Client, start: NaiveDateTime, end: NaiveDateTime) -> Result<Vec<String>> {
    let rows = conn
        .query(
            "SELECT DISTINCT national_grid_bm_unit FROM pn \
             WHERE time_to >= $1 AND time_from < $2",
            &[&start, &end],
        )
        .await
        .context("fetch active bmus")?;
    Ok(rows
        .iter()
        .filter_map(|r| r.get::<_, Option<String>>(0))
        .collect())
}

/// Compute and stage dispatch rows for a chunk of units (one worker's share).
async fn dispatch_rows(
    conn: &Client,
    units: &[String],
    start: NaiveDateTime,
    end: NaiveDateTime,
    freq_minutes: i64,
) -> Result<Vec<Row>> {
    let start = floor_to_freq(start, freq_minutes);
    let idx = grid(start, end, freq_minutes);
    let mut rows = Vec::new();
    for unit in units {
        rows.extend(bmu_dispatch_unit(conn, unit, start, end, &idx).await?);
    }
    Ok(rows)
}

/// Port of `analytics.bmu_dispatch` for a single unit: interpolate PN/BOALF/MELS
/// onto the grid and combine elementwise into `bmu_dispatch` rows.
async fn bmu_dispatch_unit(
    conn: &Client,
    unit: &str,
    start: NaiveDateTime,
    end: NaiveDateTime,
    idx: &[NaiveDateTime],
) -> Result<Vec<Row>> {
    let (pn_segs, pn_keys) = fetch_pn(conn, unit, start, end).await?;
    let boa_rows = fetch_boalf(conn, unit, start, end).await?;
    let (boa_segs, boa_keys) = boa_segments(&boa_rows);
    let (boa_so_segs, boa_so_keys) = {
        let so: Vec<BoalfSeg> = boa_rows
            .iter()
            .filter(|s| s.so_flag == Some(true))
            .cloned()
            .collect();
        boa_segments(&so)
    };
    let (mel_segs, mel_keys) = fetch_mels(conn, unit, start, end).await?;

    let pn_raw = interp_segments(&pn_segs, &pn_keys, idx);
    let boa_all = interp_segments(&boa_segs, &boa_keys, idx);
    let boa_so = interp_segments(&boa_so_segs, &boa_so_keys, idx);
    let mel_series = interp_segments(&mel_segs, &mel_keys, idx);

    let p = idx.len();
    let mut rows = Vec::new();
    for j in 0..p {
        let mel = mel_series[j];
        let pn_raw_j = pn_raw[j];
        // MEL is a physical cap; where unknown the value is left uncapped.
        let pn = if mel.is_nan() { pn_raw_j } else { nan_min(pn_raw_j, mel) };
        let boa_all_j = boa_all[j];
        let boa_capped = if mel.is_nan() { boa_all_j } else { nan_min(boa_all_j, mel) };
        let boa_so_j = boa_so[j];
        let boa_so_capped = if mel.is_nan() { boa_so_j } else { nan_min(boa_so_j, mel) };
        // Where no acceptance is active, dispatched level == FPN.
        let boa_level = if boa_capped.is_nan() { pn } else { boa_capped };

        let delta = boa_level - pn;
        let so_delta = boa_so_capped - pn;
        // clip(lower=0) propagates NaN; only so_curtailment gets fillna(0).
        let so_turnup = if delta.is_nan() { f64::NAN } else { delta.max(0.0) };
        let boa_curt = if delta.is_nan() { f64::NAN } else { (-delta).max(0.0) };
        let so_curt = if so_delta.is_nan() { 0.0 } else { (-so_delta).max(0.0) };

        // dropna(subset=["pn_mw","boa_level_mw"], how="all")
        if pn.is_nan() && boa_level.is_nan() {
            continue;
        }
        rows.push(vec![
            Value::text(unit.to_string()),
            Value::ts(idx[j]),
            float_or_null(pn),
            float_or_null(boa_level),
            float_or_null(mel),
            Value::Float(so_turnup),
            Value::Float(boa_curt),
            Value::Float(so_curt),
        ]);
    }
    Ok(rows)
}

/// Port of `analytics._fetch_pn` for a single unit.
async fn fetch_pn(
    conn: &Client,
    unit: &str,
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> Result<(Vec<Segment>, Vec<Vec<SortKey>>)> {
    let rows = conn
        .query(
            "SELECT time_from, time_to, level_from, level_to \
             FROM pn \
             WHERE national_grid_bm_unit = $1 AND time_to >= $2 AND time_from < $3",
            &[&unit, &start, &end],
        )
        .await
        .context("fetch pn")?;
    let mut segs = Vec::with_capacity(rows.len());
    let mut keys = Vec::with_capacity(rows.len());
    for r in rows {
        let t_from: NaiveDateTime = r.get(0);
        // NULL time_to: numpy `pts < NaT` is False everywhere, so the segment
        // never covers a grid point. (The fetch filter `time_to >= start` also
        // excludes NULLs; keep the guard for parity with the Python semantics.)
        let Some(t_to) = r.get::<_, Option<NaiveDateTime>>(1) else { continue };
        segs.push(Segment {
            time_from: t_from,
            time_to: t_to,
            level_from: level_or_nan(r.get::<_, Option<f64>>(2)),
            level_to: level_or_nan(r.get::<_, Option<f64>>(3)),
        });
        keys.push(vec![SortKey::Time(t_from)]);
    }
    Ok((segs, keys))
}

/// Port of `analytics._fetch_boalf` for a single unit.
async fn fetch_boalf(
    conn: &Client,
    unit: &str,
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> Result<Vec<BoalfSeg>> {
    let rows = conn
        .query(
            "SELECT acceptance_time, time_from, time_to, level_from, level_to, so_flag \
             FROM boalf \
             WHERE (ngc_bm_unit = $1 OR bm_unit = $1 OR bm_unit = 'T_' || $1) \
               AND time_to >= $2 AND time_from < $3",
            &[&unit, &start, &end],
        )
        .await
        .context("fetch boalf")?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let Some(t_to) = r.get::<_, Option<NaiveDateTime>>(2) else { continue };
        out.push(BoalfSeg {
            acceptance_time: r.get(0),
            time_from: r.get(1),
            time_to: t_to,
            level_from: level_or_nan(r.get::<_, Option<f64>>(3)),
            level_to: level_or_nan(r.get::<_, Option<f64>>(4)),
            so_flag: r.get(5),
        });
    }
    Ok(out)
}

/// Port of `analytics._fetch_mels` for a single unit.
async fn fetch_mels(
    conn: &Client,
    unit: &str,
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> Result<(Vec<Segment>, Vec<Vec<SortKey>>)> {
    let rows = conn
        .query(
            "SELECT notification_sequence, time_from, time_to, level_from, level_to \
             FROM mels \
             WHERE national_grid_bm_unit = $1 AND time_to >= $2 AND time_from < $3",
            &[&unit, &start, &end],
        )
        .await
        .context("fetch mels")?;
    let mut segs = Vec::with_capacity(rows.len());
    let mut keys = Vec::with_capacity(rows.len());
    for r in rows {
        let seq: i64 = r.get(0);
        let t_from: NaiveDateTime = r.get(1);
        let Some(t_to) = r.get::<_, Option<NaiveDateTime>>(2) else { continue };
        segs.push(Segment {
            time_from: t_from,
            time_to: t_to,
            level_from: level_or_nan(r.get::<_, Option<f64>>(3)),
            level_to: level_or_nan(r.get::<_, Option<f64>>(4)),
        });
        keys.push(vec![SortKey::Int(seq), SortKey::Time(t_from)]);
    }
    Ok((segs, keys))
}

/// Build the BOALF segment/key lists with order keys
/// `[acceptance_time, time_from]`. A NULL acceptance_time is represented by a
/// far-future sentinel so it sorts *after* any real acceptance_time — pandas
/// `sort_values` puts NaT last (`na_position='last'`, verified empirically), so
/// the NULL-acceptance_time row wins overlaps, exactly as the reference does.
fn boa_segments(rows: &[BoalfSeg]) -> (Vec<Segment>, Vec<Vec<SortKey>>) {
    let mut segs = Vec::with_capacity(rows.len());
    let mut keys = Vec::with_capacity(rows.len());
    for s in rows {
        segs.push(Segment {
            time_from: s.time_from,
            time_to: s.time_to,
            level_from: s.level_from,
            level_to: s.level_to,
        });
        keys.push(vec![
            SortKey::Time(s.acceptance_time.unwrap_or(NaiveDateTime::MAX)),
            SortKey::Time(s.time_from),
        ]);
    }
    (segs, keys)
}

/// Floor `ts` down to the nearest `freq_minutes` boundary (port of
/// `pd.Timestamp(ts).floor(freq)`), so timestamps always land on the fixed grid.
fn floor_to_freq(ts: NaiveDateTime, freq_minutes: i64) -> NaiveDateTime {
    let step_ns = freq_minutes * 60_000_000_000_i64;
    let secs = ts.and_utc().timestamp();
    let nanos = i64::from(ts.and_utc().timestamp_subsec_nanos());
    let total_ns = secs.saturating_mul(1_000_000_000).saturating_add(nanos);
    let floored = total_ns - total_ns.rem_euclid(step_ns);
    let fsecs = floored.div_euclid(1_000_000_000);
    let fnanos = floored.rem_euclid(1_000_000_000) as u32;
    chrono::DateTime::from_timestamp(fsecs, fnanos)
        .expect("floored timestamp in range")
        .naive_utc()
}

/// `date_range(start, end, freq, inclusive="left")`: points < `end`.
fn grid(start: NaiveDateTime, end: NaiveDateTime, freq_minutes: i64) -> Vec<NaiveDateTime> {
    let step = chrono::Duration::minutes(freq_minutes);
    let mut pts = Vec::new();
    let mut t = start;
    while t < end {
        pts.push(t);
        match t.checked_add_signed(step) {
            Some(x) => t = x,
            None => break,
        }
    }
    pts
}

/// `np.minimum(a, b)` — NaN propagates.
fn nan_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.min(b)
    }
}

/// NaN → NULL (pn/boa/mel columns); the derived columns keep raw f64 instead.
fn float_or_null(v: f64) -> Value {
    if v.is_nan() {
        Value::Null
    } else {
        Value::Float(v)
    }
}

/// SQL NULL level → NaN (pandas reads NULL as NaN).
fn level_or_nan(v: Option<f64>) -> f64 {
    v.unwrap_or(f64::NAN)
}

pub async fn materialize_dispatch_daily(
    cfg: &Config,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<usize> {
    // Verbatim port of `materialize.py` `materialize_dispatch_daily` (params
    // renamed %(h)s/%(start)s/%(end)s/%(start_d)s/%(end_d)s → $1..$5).
    const SQL: &str = r#"
    WITH dispatch AS (
        SELECT bmu,
               bucket::date AS date,
               pn_mw_sum     * $1 AS pn_mwh,
               boa_mw_sum    * $1 AS boa_dispatched_mwh,
               so_turnup_sum * $1 AS so_turnup_mwh,
               boa_curt_sum  * $1 AS boa_curtailment_mwh,
               so_curt_sum   * $1 AS so_curtailment_mwh
        FROM bmu_dispatch_daily_cagg
        WHERE bucket >= $2 AND bucket < $3
    ),
    actual AS (
        SELECT bmu,
               bucket AS date,
               b1610_mwh
        FROM b1610_daily_cagg
        WHERE bucket >= $4 AND bucket <= $5
    )
    INSERT INTO bmu_dispatch_daily (
        bmu, date, pn_mwh, boa_dispatched_mwh,
        so_turnup_mwh, boa_curtailment_mwh, so_curtailment_mwh, b1610_mwh
    )
    SELECT COALESCE(d.bmu, a.bmu)              AS bmu,
           COALESCE(d.date, a.date)            AS date,
           d.pn_mwh, d.boa_dispatched_mwh,
           d.so_turnup_mwh, d.boa_curtailment_mwh, d.so_curtailment_mwh,
           a.b1610_mwh
    FROM dispatch d
    FULL OUTER JOIN actual a ON a.bmu = d.bmu AND a.date = d.date
    ON CONFLICT (bmu, date) DO UPDATE SET
        pn_mwh              = EXCLUDED.pn_mwh,
        boa_dispatched_mwh  = EXCLUDED.boa_dispatched_mwh,
        so_turnup_mwh       = EXCLUDED.so_turnup_mwh,
        boa_curtailment_mwh = EXCLUDED.boa_curtailment_mwh,
        so_curtailment_mwh  = EXCLUDED.so_curtailment_mwh,
        b1610_mwh           = EXCLUDED.b1610_mwh
    "#;
    let start_dt = start.and_hms_opt(0, 0, 0).unwrap();
    let end_dt = (end + Days::new(1)).and_hms_opt(0, 0, 0).unwrap();
    let h = DAILY_INTERVAL_HOURS;
    let conn = db::connect(cfg).await?;
    let n = conn
        .execute(SQL, &[&h, &start_dt, &end_dt, &start, &end])
        .await
        .context("materialize_dispatch_daily")?;
    crate::log_info!(
        "materialize_dispatch_daily_wrote",
        "rows", n,
        "start", start,
        "end", end
    );
    Ok(n as usize)
}

pub async fn cmd_materialize_dispatch(
    cfg: &Config,
    from: &str,
    to: &str,
    bmus: &[String],
    workers: Option<usize>,
) -> Result<()> {
    let from_d = cli_date(from)?;
    let to_d = cli_date(to)?;
    let start = from_d.and_hms_opt(0, 0, 0).unwrap();
    let end = to_d
        .and_time(NaiveTime::from_hms_nano_opt(23, 59, 59, 999_999_000).unwrap());
    let bmus_opt = if bmus.is_empty() { None } else { Some(bmus) };
    let n = materialize_dispatch(cfg, start, end, bmus_opt, workers, 5).await?;
    println!("wrote {n} rows");
    Ok(())
}

pub async fn cmd_materialize_dispatch_daily(cfg: &Config, from: &str, to: &str) -> Result<()> {
    let from_d = cli_date(from)?;
    let to_d = cli_date(to)?;
    let n = materialize_dispatch_daily(cfg, from_d, to_d).await?;
    println!("wrote {n} rows");
    Ok(())
}