use crate::error::AppError;
use duckdb::Connection;
use serde_json::Value;

/// Fetch all column metadata for the data view.
pub fn get_columns(conn: &Connection) -> Result<Vec<(String, String)>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT column_name, data_type FROM information_schema.columns \
         WHERE table_name = 'data' ORDER BY ordinal_position",
    )?;
    let cols: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(cols)
}

/// Get the row count from the data view.
pub fn row_count(conn: &Connection) -> Result<i64, AppError> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM data", [], |row| row.get(0))?;
    Ok(count)
}

/// Fetch rows by a list of rowids, returning them as JSON values.
pub fn fetch_rows_by_ids(
    conn: &Connection,
    rowids: &[i64],
    columns: Option<&[String]>,
) -> Result<Vec<Value>, AppError> {
    if rowids.is_empty() {
        return Ok(vec![]);
    }

    let col_clause = match columns {
        Some(cols) if !cols.is_empty() => cols.join(", "),
        _ => "*".to_string(),
    };

    let placeholders: Vec<String> = rowids.iter().map(|id| id.to_string()).collect();
    let sql = format!(
        "SELECT {} FROM data WHERE rowid IN ({})",
        col_clause,
        placeholders.join(", ")
    );

    execute_query_to_json(conn, &sql)
}

/// Fetch all rows with optional SQL clauses.
pub fn fetch_all_rows(
    conn: &Connection,
    where_clauses: &[(String, String)],
    select_cols: Option<&[String]>,
    group_by: Option<&str>,
    agg: Option<&str>,
    limit: usize,
) -> Result<Vec<Value>, AppError> {
    let col_clause = build_select_clause(select_cols, group_by, agg);
    let mut sql = format!("SELECT {} FROM data", col_clause);

    let mut conditions = Vec::new();
    for (col, val) in where_clauses {
        conditions.push(format!("{} = '{}'", col, val.replace('\'', "''")));
    }
    if !conditions.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conditions.join(" AND "));
    }

    if let Some(gb) = group_by {
        sql.push_str(&format!(" GROUP BY {}", gb));
    }

    sql.push_str(&format!(" LIMIT {}", limit));

    execute_query_to_json(conn, &sql)
}

/// Execute an arbitrary SQL query and return rows as JSON values.
/// Gets column names from information_schema to avoid the DuckDB prepared statement limitation.
pub fn execute_query_to_json(conn: &Connection, sql: &str) -> Result<Vec<Value>, AppError> {
    // First, get column names by wrapping the query as a subquery with LIMIT 0
    // and using DESCRIBE, or we can get them from the first row.
    // Safest approach: use a wrapper query to get column names
    let col_names = get_query_column_names(conn, sql)?;
    let mut stmt = conn.prepare(sql)?;
    let rows: Vec<Value> = stmt
        .query_map([], |row| {
            let mut map = serde_json::Map::new();
            for (i, name) in col_names.iter().enumerate() {
                let val = duckdb_value_to_json(row, i);
                map.insert(name.clone(), val);
            }
            Ok(Value::Object(map))
        })?
        .filter_map(|r| r.ok())
        .collect();

    Ok(rows)
}

/// Get column names for a query by creating a temp view and inspecting it.
fn get_query_column_names(conn: &Connection, sql: &str) -> Result<Vec<String>, AppError> {
    let view_name = format!("_col_inspect_{}", uuid::Uuid::new_v4().to_string().replace('-', ""));
    conn.execute_batch(&format!(
        "CREATE TEMPORARY VIEW {} AS {}",
        view_name, sql
    ))?;

    let mut stmt = conn.prepare(&format!(
        "SELECT column_name FROM information_schema.columns WHERE table_name = '{}' ORDER BY ordinal_position",
        view_name
    ))?;
    let names: Vec<String> = stmt
        .query_map([], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();

    conn.execute_batch(&format!("DROP VIEW IF EXISTS {}", view_name))?;

    Ok(names)
}

fn build_select_clause(
    select_cols: Option<&[String]>,
    group_by: Option<&str>,
    agg: Option<&str>,
) -> String {
    match (group_by, agg) {
        (Some(gb), Some(a)) => {
            let agg_expr = if a == "count" {
                "COUNT(*) AS count".to_string()
            } else if let Some(col) = a.strip_prefix("sum:") {
                format!("SUM({}) AS sum_{}", col, col)
            } else if let Some(col) = a.strip_prefix("avg:") {
                format!("AVG({}) AS avg_{}", col, col)
            } else if let Some(col) = a.strip_prefix("min:") {
                format!("MIN({}) AS min_{}", col, col)
            } else if let Some(col) = a.strip_prefix("max:") {
                format!("MAX({}) AS max_{}", col, col)
            } else {
                "COUNT(*) AS count".to_string()
            };
            format!("{}, {}", gb, agg_expr)
        }
        _ => match select_cols {
            Some(cols) if !cols.is_empty() => cols.join(", "),
            _ => "*".to_string(),
        },
    }
}

fn duckdb_value_to_json(row: &duckdb::Row, idx: usize) -> Value {
    let dv: duckdb::types::Value = match row.get(idx) {
        Ok(v) => v,
        Err(_) => return Value::Null,
    };
    duckdb_val_to_json(dv)
}

fn duckdb_val_to_json(dv: duckdb::types::Value) -> Value {
    use duckdb::types::Value as DV;
    match dv {
        DV::Null => Value::Null,
        DV::Boolean(b) => Value::Bool(b),
        DV::TinyInt(v) => Value::Number((v as i64).into()),
        DV::SmallInt(v) => Value::Number((v as i64).into()),
        DV::Int(v) => Value::Number((v as i64).into()),
        DV::BigInt(v) => Value::Number(v.into()),
        DV::HugeInt(v) => Value::Number((v as i64).into()),
        DV::UTinyInt(v) => Value::Number((v as i64).into()),
        DV::USmallInt(v) => Value::Number((v as i64).into()),
        DV::UInt(v) => Value::Number((v as i64).into()),
        DV::UBigInt(v) => Value::Number(v.into()),
        DV::Float(v) => serde_json::Number::from_f64(v as f64)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        DV::Double(v) => serde_json::Number::from_f64(v)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        DV::Decimal(d) => {
            Value::String(d.to_string())
        }
        DV::Text(s) => Value::String(s),
        DV::Blob(b) => Value::String(format!("<blob {} bytes>", b.len())),
        DV::Timestamp(_, micros) => {
            // Convert microseconds since epoch to ISO string
            let secs = micros / 1_000_000;
            let nsecs = ((micros % 1_000_000) * 1000) as u32;
            Value::String(format!("{}",
                chrono_from_epoch(secs, nsecs)
            ))
        }
        DV::Date32(days) => {
            // Days since Unix epoch
            let date = epoch_days_to_date(days);
            Value::String(date)
        }
        DV::Time64(_, v) => Value::String(format!("{}", v)),
        DV::Interval { months, days, nanos } => {
            Value::String(format!("{}m{}d{}ns", months, days, nanos))
        }
        DV::List(items) => {
            let arr: Vec<Value> = items.into_iter().map(duckdb_val_to_json).collect();
            Value::Array(arr)
        }
        DV::Enum(s) => Value::String(s),
        DV::Struct(_fields) => {
            Value::String("<struct>".to_string())
        }
        DV::Union(v) => duckdb_val_to_json(*v),
        DV::Map(_m) => {
            Value::String("<map>".to_string())
        }
        DV::Array(items) => {
            let arr: Vec<Value> = items.into_iter().map(duckdb_val_to_json).collect();
            Value::Array(arr)
        }
    }
}

fn epoch_days_to_date(days: i32) -> String {
    // civil_from_days expects days since Unix epoch (1970-01-01)
    let (y, m, d) = civil_from_days(days as i64);
    format!("{:04}-{:02}-{:02}", y, m, d)
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    // Algorithm from Howard Hinnant's date algorithms
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

fn chrono_from_epoch(secs: i64, _nsecs: u32) -> String {
    let days = secs / 86400;
    let rem = secs % 86400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (y, mo, d) = civil_from_days(days);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, mo, d, h, m, s)
}
