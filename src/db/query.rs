use crate::db;
use crate::error::AppError;
use chrono::NaiveDate;
use duckdb::arrow::datatypes::DataType;
use duckdb::Connection;
use serde_json::{json, Value};
use std::sync::Mutex;

/// Fetch rows by rowid list. Returns all columns as JSON objects.
pub fn get_rows_by_ids(db: &Mutex<Connection>, ids: &[i64]) -> Result<Vec<Value>, AppError> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let placeholders = ids.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(",");
    let sql = format!("SELECT * FROM data WHERE rowid IN ({}) ORDER BY rowid", placeholders);
    execute_query_to_json(db, &sql)
}

/// Execute a query with optional spatial rowid filter, where/select/group_by/agg/limit.
pub fn query(
    db: &Mutex<Connection>,
    ids: Option<&[i64]>,
    where_clauses: &[(String, String)],
    select_cols: Option<&[String]>,
    group_by: Option<&str>,
    agg: Option<&str>,
    limit: usize,
) -> Result<Vec<Value>, AppError> {
    // Aggregation path
    if let (Some(gb), Some(a)) = (group_by, agg) {
        db::validate_column_name(gb)?;
        return query_aggregate(db, ids, where_clauses, gb, a, limit);
    }

    // Select clause
    let select = match select_cols {
        Some(cols) => {
            for c in cols {
                db::validate_column_name(c)?;
            }
            cols.join(", ")
        }
        None => "*".to_string(),
    };

    let mut sql = format!("SELECT {} FROM data", select);
    let mut conditions = Vec::new();

    // Rowid filter
    if let Some(ids) = ids {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let placeholders = ids.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(",");
        conditions.push(format!("rowid IN ({})", placeholders));
    }

    // Where clauses
    for (col, val) in where_clauses {
        db::validate_column_name(col)?;
        conditions.push(format!("CAST(\"{}\" AS VARCHAR) = '{}'", col, escape_sql_value(val)));
    }

    if !conditions.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conditions.join(" AND "));
    }

    // Preserve rowid ordering when we have spatial candidates
    if ids.is_some() {
        // No extra ORDER BY — the IN list order isn't guaranteed but that's fine,
        // callers that need ordering (radius, nearest) sort after injection.
    }

    sql.push_str(&format!(" LIMIT {}", limit));

    execute_query_to_json(db, &sql)
}

fn query_aggregate(
    db: &Mutex<Connection>,
    ids: Option<&[i64]>,
    where_clauses: &[(String, String)],
    group_by: &str,
    agg: &str,
    limit: usize,
) -> Result<Vec<Value>, AppError> {
    let agg_expr = if agg == "count" {
        "COUNT(*) AS count".to_string()
    } else if let Some(col) = agg.strip_prefix("sum:") {
        db::validate_column_name(col)?;
        format!("SUM(\"{}\") AS sum_{}", col, col)
    } else if let Some(col) = agg.strip_prefix("avg:") {
        db::validate_column_name(col)?;
        format!("AVG(\"{}\") AS avg_{}", col, col)
    } else if let Some(col) = agg.strip_prefix("min:") {
        db::validate_column_name(col)?;
        format!("MIN(\"{}\") AS min_{}", col, col)
    } else if let Some(col) = agg.strip_prefix("max:") {
        db::validate_column_name(col)?;
        format!("MAX(\"{}\") AS max_{}", col, col)
    } else {
        "COUNT(*) AS count".to_string()
    };

    let mut sql = format!(
        "SELECT \"{}\", {} FROM data",
        group_by, agg_expr
    );

    let mut conditions = Vec::new();

    if let Some(ids) = ids {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let placeholders = ids.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(",");
        conditions.push(format!("rowid IN ({})", placeholders));
    }

    for (col, val) in where_clauses {
        db::validate_column_name(col)?;
        conditions.push(format!("CAST(\"{}\" AS VARCHAR) = '{}'", col, escape_sql_value(val)));
    }

    if !conditions.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conditions.join(" AND "));
    }

    sql.push_str(&format!(" GROUP BY \"{}\"", group_by));
    sql.push_str(&format!(" LIMIT {}", limit));

    execute_query_to_json(db, &sql)
}

/// Execute a SQL query and return results as Vec<serde_json::Value>.
fn execute_query_to_json(db: &Mutex<Connection>, sql: &str) -> Result<Vec<Value>, AppError> {
    let conn = db.lock().unwrap();
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("SQL prepare error: {} — {}", e, sql)))?;

    let mut rows = stmt
        .query([])
        .map_err(|e| AppError::Internal(anyhow::anyhow!("SQL query error: {} — {}", e, sql)))?;

    // Column names and types are available only after query execution in duckdb crate
    let stmt_ref = rows.as_ref().unwrap();
    let col_count = stmt_ref.column_count();
    let col_names: Vec<String> = (0..col_count)
        .map(|i| stmt_ref.column_name(i).map_or("?".to_string(), |v| v.to_string()))
        .collect();
    let col_types: Vec<DataType> = (0..col_count)
        .map(|i| stmt_ref.column_type(i))
        .collect();

    let mut rows_out = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Row fetch error: {}", e)))?
    {
        let mut obj = serde_json::Map::with_capacity(col_count);
        for (i, name) in col_names.iter().enumerate() {
            let val = row_value_to_json(row, i, &col_types[i]);
            obj.insert(name.clone(), val);
        }
        rows_out.push(Value::Object(obj));
    }

    Ok(rows_out)
}

/// Extract a DuckDB row value at column index into a serde_json::Value,
/// using the Arrow DataType to pick the right conversion.
fn row_value_to_json(row: &duckdb::Row, idx: usize, col_type: &DataType) -> Value {
    match col_type {
        DataType::Int8 | DataType::Int16 | DataType::Int32 | DataType::Int64
        | DataType::UInt8 | DataType::UInt16 | DataType::UInt32 | DataType::UInt64 => {
            match row.get::<_, i64>(idx) {
                Ok(v) => json!(v),
                Err(_) => Value::Null,
            }
        }
        DataType::Float16 | DataType::Float32 | DataType::Float64 | DataType::Decimal128(_, _) | DataType::Decimal256(_, _) => {
            match row.get::<_, f64>(idx) {
                Ok(v) if v.is_nan() || v.is_infinite() => Value::Null,
                Ok(v) => json!(v),
                Err(_) => Value::Null,
            }
        }
        DataType::Boolean => {
            match row.get::<_, bool>(idx) {
                Ok(v) => json!(v),
                Err(_) => Value::Null,
            }
        }
        DataType::Date32 => {
            // Date32 = days since Unix epoch
            match row.get::<_, i32>(idx) {
                Ok(days) => {
                    let date = NaiveDate::from_num_days_from_ce_opt(days + 719_163)
                        .map(|d| d.format("%Y-%m-%d").to_string());
                    match date {
                        Some(s) => json!(s),
                        None => json!(days),
                    }
                }
                Err(_) => Value::Null,
            }
        }
        DataType::Date64 | DataType::Timestamp(_, _) => {
            // Try string representation
            match row.get::<_, String>(idx) {
                Ok(v) => json!(v),
                Err(_) => {
                    // Fallback: try as i64 (microseconds/milliseconds since epoch)
                    match row.get::<_, i64>(idx) {
                        Ok(v) => json!(v),
                        Err(_) => Value::Null,
                    }
                }
            }
        }
        _ => {
            // For Utf8 (VARCHAR) and everything else — get as string
            match row.get::<_, String>(idx) {
                Ok(v) => json!(v),
                Err(_) => Value::Null,
            }
        }
    }
}

/// Scan lat/lon columns from DuckDB for R-tree building.
/// Returns Vec<(rowid, lat, lon)>.
pub fn scan_lat_lon(
    db: &Mutex<Connection>,
    lat_col: &str,
    lon_col: &str,
) -> Result<Vec<(i64, f64, f64)>, AppError> {
    let conn = db.lock().unwrap();
    let sql = format!(
        "SELECT rowid, \"{}\", \"{}\" FROM data WHERE \"{}\" IS NOT NULL AND \"{}\" IS NOT NULL",
        lat_col, lon_col, lat_col, lon_col
    );
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("scan_lat_lon prepare: {}", e)))?;
    let mut rows = stmt
        .query([])
        .map_err(|e| AppError::Internal(anyhow::anyhow!("scan_lat_lon query: {}", e)))?;

    let mut points = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("scan_lat_lon row: {}", e)))?
    {
        let rowid: i64 = row.get(0).unwrap();
        let lat: f64 = row.get(1).unwrap();
        let lon: f64 = row.get(2).unwrap();
        points.push((rowid, lat, lon));
    }

    Ok(points)
}

fn escape_sql_value(s: &str) -> String {
    s.replace('\'', "''")
}
