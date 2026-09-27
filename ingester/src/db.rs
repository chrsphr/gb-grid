//! Postgres access: connection, upsert (VALUES + COPY-into-staging paths),
//! watermark tracking, and continuous-aggregate refresh.
//!
//! Mirrors `src/gb_grid/db.py`. Timestamps are stored as naive UTC (`TIMESTAMP`
//! columns); dates as `DATE`. `COPY_MIN_ROWS` keeps small batches on the VALUES
//! path, where the staging round-trip would otherwise dominate.

use std::error::Error;
use std::pin::pin;

use anyhow::{Context, Result};
use bytes::Bytes;
use chrono::{NaiveDate, NaiveDateTime};
use futures::SinkExt;
use tokio_postgres::types::private::BytesMut;
use tokio_postgres::types::{IsNull, ToSql, Type};
use tokio_postgres::{Client, NoTls};

use crate::config::{Config, COPY_MIN_ROWS};

/// A single cell value in an ingest row. Ordered column names are carried
/// separately (see [`upsert`]) — every row in a batch shares the same columns.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    Date(NaiveDate),
    Timestamp(NaiveDateTime),
}

impl Value {
    pub fn text(s: impl Into<String>) -> Value {
        Value::Text(s.into())
    }
    pub fn ts(dt: NaiveDateTime) -> Value {
        Value::Timestamp(dt)
    }
    pub fn f(f: f64) -> Value {
        Value::Float(f)
    }
}

pub type Row = Vec<Value>;

impl ToSql for Value {
    fn to_sql(&self, ty: &Type, out: &mut BytesMut) -> Result<IsNull, Box<dyn Error + Sync + Send>> {
        match self {
            Value::Null => Ok(IsNull::Yes),
            Value::Bool(b) => b.to_sql(ty, out),
            Value::Int(i) => {
                // Columns vary in width (settlement_period is SMALLINT/int2,
                // acceptance_id is BIGINT/int8); the i64 impl always formats
                // int8 and Postgres rejects it for narrower targets.
                if *ty == Type::INT2 {
                    (*i as i16).to_sql(ty, out)
                } else if *ty == Type::INT4 {
                    (*i as i32).to_sql(ty, out)
                } else {
                    i.to_sql(ty, out)
                }
            }
            Value::Float(f) => {
                if *ty == Type::FLOAT4 {
                    (*f as f32).to_sql(ty, out)
                } else {
                    f.to_sql(ty, out)
                }
            }
            Value::Text(s) => s.to_sql(ty, out),
            Value::Date(d) => d.to_sql(ty, out),
            Value::Timestamp(t) => t.to_sql(ty, out),
        }
    }

    fn accepts(_ty: &Type) -> bool {
        // The real impls are delegated to above, which format according to the
        // type Postgres advertises for each bind parameter.
        true
    }

    tokio_postgres::types::to_sql_checked!();
}

pub async fn connect(cfg: &Config) -> Result<Client> {
    let (client, connection) = tokio_postgres::connect(&cfg.database_url, NoTls)
        .await
        .context("connect to postgres")?;
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("postgres connection error: {e}");
        }
    });
    Ok(client)
}

/// Bulk upsert. Small batches go via a single multi-row VALUES statement;
/// large ones via COPY into an unlogged staging table plus one merge.
pub async fn upsert(
    conn: &Client,
    table: &str,
    cols: &[String],
    rows: &[Row],
    conflict_cols: &[&str],
) -> Result<usize> {
    if rows.is_empty() {
        return Ok(0);
    }
    if rows.len() < COPY_MIN_ROWS {
        upsert_values(conn, table, cols, rows, conflict_cols).await
    } else {
        upsert_copy(conn, table, cols, rows, conflict_cols).await
    }
}

fn conflict_tail(cols: &[String], conflict_cols: &[&str]) -> String {
    let conflict = conflict_cols.join(", ");
    let update_cols: Vec<&String> = cols.iter().filter(|c| !conflict_cols.contains(&c.as_str())).collect();
    if update_cols.is_empty() {
        format!("ON CONFLICT ({conflict}) DO NOTHING")
    } else {
        let set_clause = update_cols
            .iter()
            .map(|c| format!("{c} = EXCLUDED.{c}"))
            .collect::<Vec<_>>()
            .join(", ");
        format!("ON CONFLICT ({conflict}) DO UPDATE SET {set_clause}")
    }
}

/// Collision-free string key for one row's conflict columns (kept within a
/// single batch, so ordering is stable).
fn value_key(v: &Value) -> String {
    match v {
        Value::Null => "n".to_string(),
        Value::Bool(b) => if *b { "b1".into() } else { "b0".into() },
        Value::Int(i) => format!("i{i}"),
        Value::Float(f) => format!("f{:016x}", f.to_bits()),
        Value::Text(s) => format!("t{s}"),
        Value::Date(d) => format!("d{}", d.format("%Y%m%d")),
        Value::Timestamp(t) => format!("s{}", t.and_utc().timestamp_nanos_opt().unwrap_or(0)),
    }
}

/// Deduplicate within a batch keeping the *last* occurrence per key. `executemany`
/// applies rows one statement at a time so repeated keys overwrite in order; a
/// single INSERT..SELECT (COPY path) or one multi-row VALUES statement cannot
/// touch the same row twice, so duplicates must be collapsed first.
fn dedup_last(rows: &[Row], cols: &[String], conflict_cols: &[&str]) -> Vec<Row> {
    let conflict_idx: Vec<usize> = conflict_cols
        .iter()
        .map(|c| cols.iter().position(|x| x == c).expect("conflict col present"))
        .collect();
    let mut last: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (i, row) in rows.iter().enumerate() {
        let key = conflict_idx
            .iter()
            .map(|&ci| value_key(&row[ci]))
            .collect::<Vec<_>>()
            .join("|");
        last.insert(key, i);
    }
    let mut out: Vec<Row> = Vec::with_capacity(last.len());
    let mut indices: Vec<usize> = last.into_values().collect();
    indices.sort_unstable();
    for i in indices {
        out.push(rows[i].clone());
    }
    out
}

async fn upsert_values(
    conn: &Client,
    table: &str,
    cols: &[String],
    rows: &[Row],
    conflict_cols: &[&str],
) -> Result<usize> {
    let deduped = dedup_last(rows, cols, conflict_cols);
    let col_list = cols.join(", ");
    let mut params: Vec<&(dyn ToSql + Sync)> = Vec::new();
    let mut groups: Vec<String> = Vec::with_capacity(deduped.len());
    for row in &deduped {
        let ph: Vec<String> = (1..=cols.len())
            .map(|i| format!("${}", i + params.len()))
            .collect();
        groups.push(format!("({})", ph.join(", ")));
        params.extend(row.iter().map(|v| v as &(dyn ToSql + Sync)));
    }
    let sql = format!(
        "INSERT INTO {table} ({col_list}) VALUES {} {}",
        groups.join(", "),
        conflict_tail(cols, conflict_cols)
    );
    conn.execute(&sql, &params).await.context("VALUES upsert")?;
    Ok(deduped.len())
}

fn encode_copy(v: &Value, out: &mut Vec<u8>) {
    match v {
        Value::Null => out.extend_from_slice(b"\\N"),
        Value::Bool(true) => out.push(b't'),
        Value::Bool(false) => out.push(b'f'),
        Value::Int(i) => out.extend_from_slice(i.to_string().as_bytes()),
        Value::Float(f) => {
            let s = if f.is_nan() {
                "NaN".to_string()
            } else if f.is_infinite() {
                if *f > 0.0 { "Infinity".to_string() } else { "-Infinity".to_string() }
            } else {
                f.to_string()
            };
            out.extend_from_slice(s.as_bytes());
        }
        Value::Text(s) => {
            for b in s.bytes() {
                match b {
                    b'\\' => out.extend_from_slice(b"\\\\"),
                    b'\t' => out.extend_from_slice(b"\\t"),
                    b'\n' => out.extend_from_slice(b"\\n"),
                    b'\r' => out.extend_from_slice(b"\\r"),
                    _ => out.push(b),
                }
            }
        }
        Value::Date(d) => out.extend_from_slice(d.format("%Y-%m-%d").to_string().as_bytes()),
        Value::Timestamp(t) => {
            out.extend_from_slice(t.format("%Y-%m-%d %H:%M:%S%.6f").to_string().as_bytes())
        }
    }
}

async fn upsert_copy(
    conn: &Client,
    table: &str,
    cols: &[String],
    rows: &[Row],
    conflict_cols: &[&str],
) -> Result<usize> {
    let stage = format!("_stage_{table}");
    let col_list = cols.join(", ");
    let conflict = conflict_cols.join(", ");

    // No wrapping transaction (the tokio-postgres `Client` is not `Clone` and
    // `transaction()` needs `&mut`): the staging temp table is session-scoped
    // and explicitly dropped, and the final merge into the real table is one
    // atomic statement, so a crash mid-copy can't leave partial data behind.
    conn.batch_execute(&format!(
        "CREATE TEMP TABLE {stage} (LIKE {table}); \
         ALTER TABLE {stage} ADD COLUMN _ord bigserial"
    ))
    .await?;

    let copy_sql = format!("COPY {stage} ({col_list}) FROM STDIN");
    let mut sink = pin!(conn.copy_in(&copy_sql).await?);
    let mut line: Vec<u8> = Vec::with_capacity(64);
    for row in rows {
        line.clear();
        for v in row {
            encode_copy(v, &mut line);
            line.push(b'\t');
        }
        line.pop(); // drop trailing tab
        line.push(b'\n');
        sink.as_mut().send(Bytes::from(line.clone())).await?;
    }
    // `finish` completes the COPY (sends CopyDone) and returns rows copied.
    sink.as_mut().finish().await?;

    conn.execute(
        &format!(
            "INSERT INTO {table} ({col_list}) \
             SELECT DISTINCT ON ({conflict}) {col_list} FROM {stage} \
             ORDER BY {conflict}, _ord DESC {}",
            conflict_tail(cols, conflict_cols)
        ),
        &[],
    )
    .await?;
    conn.execute(&format!("DROP TABLE {stage}"), &[]).await?;
    Ok(rows.len())
}

pub async fn set_watermark(conn: &Client, dataset: &str, last_ts: NaiveDateTime) -> Result<()> {
    conn.execute(
        "INSERT INTO ingest_watermark(dataset, last_ts, updated_at) \
         VALUES ($1, $2, CURRENT_TIMESTAMP) \
         ON CONFLICT (dataset) DO UPDATE SET \
           last_ts = EXCLUDED.last_ts, updated_at = EXCLUDED.updated_at",
        &[&dataset, &last_ts],
    )
    .await?;
    Ok(())
}

pub async fn get_watermark(conn: &Client, dataset: &str) -> Result<Option<NaiveDateTime>> {
    let row = conn
        .query_opt("SELECT last_ts FROM ingest_watermark WHERE dataset = $1", &[&dataset])
        .await?;
    Ok(row.and_then(|r| r.get::<_, Option<NaiveDateTime>>(0)))
}

/// Refresh a continuous aggregate over `[start, end)`.
///
/// `refresh_continuous_aggregate()` cannot run inside a transaction block, and
/// tokio-postgres executes statements in autocommit by default, so a bare
/// `execute` is correct here. The cagg name is an internal constant (never user
/// input) and is interpolated directly: the procedure's first argument is a
/// `regclass` and so cannot be a bind parameter. The time params are cast
/// explicitly because the procedure has overloads and Postgres cannot otherwise
/// infer `$1`'s type; naive-UTC timestamps are passed as `timestamp`, matching
/// the Python (psycopg) behaviour.
pub async fn refresh_continuous_aggregate(
    conn: &Client,
    cagg: &str,
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> Result<()> {
    conn.execute(
        &format!("CALL refresh_continuous_aggregate('{cagg}', $1::timestamp, $2::timestamp)"),
        &[&start, &end],
    )
    .await?;
    Ok(())
}

pub async fn refresh_caggs(
    conn: &Client,
    caggs: &[&str],
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> Result<()> {
    for cagg in caggs {
        refresh_continuous_aggregate(conn, cagg, start, end).await?;
    }
    Ok(())
}

/// Date-typed variant for continuous aggregates bucketed on a DATE column
/// (the b1610 caggs bucket `settlement_date`); passing a timestamp to those
/// errors with "invalid time argument type".
pub async fn refresh_continuous_aggregate_dates(
    conn: &Client,
    cagg: &str,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<()> {
    conn.execute(
        &format!("CALL refresh_continuous_aggregate('{cagg}', $1::date, $2::date)"),
        &[&start, &end],
    )
    .await?;
    Ok(())
}

pub async fn refresh_caggs_dates(
    conn: &Client,
    caggs: &[&str],
    start: NaiveDate,
    end: NaiveDate,
) -> Result<()> {
    for cagg in caggs {
        refresh_continuous_aggregate_dates(conn, cagg, start, end).await?;
    }
    Ok(())
}

pub struct TableStats {
    pub table: String,
    pub rows: i64,
    /// Rendered `max(ts_col)` (a `DATE` column like `b1610.settlement_date`
    /// would otherwise fail to deserialize as a timestamp).
    pub latest: Option<String>,
}

pub async fn table_stats(conn: &Client, table: &str, ts_col: &str) -> Result<TableStats> {
    let sql = format!("SELECT count(*)::bigint AS rows, max({ts_col})::text FROM {table}");
    let row = conn.query_one(&sql, &[]).await?;
    Ok(TableStats {
        table: table.to_string(),
        rows: row.get(0),
        latest: row.get(1),
    })
}