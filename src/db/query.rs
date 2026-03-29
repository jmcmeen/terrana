use crate::db;
use crate::error::AppError;
use chrono::NaiveDate;
use duckdb::arrow::datatypes::DataType;
use duckdb::Connection;
use serde_json::{json, Value};
use std::sync::Mutex;

// --- Spatial SQL helpers (DuckDB spatial extension) ---

/// WHERE fragment for bounding box using ST_Intersects (R-tree accelerated).
pub fn bbox_filter(min_lat: f64, min_lon: f64, max_lat: f64, max_lon: f64) -> String {
    format!(
        "ST_Intersects(geom, ST_MakeEnvelope({}, {}, {}, {}))",
        min_lon, min_lat, max_lon, max_lat
    )
}

/// WHERE fragment for radius: bbox envelope + ST_Distance_Sphere.
pub fn radius_filter(lat: f64, lon: f64, radius_m: f64) -> String {
    let deg_offset = radius_m / 111_000.0 * 1.5;
    let bbox = bbox_filter(
        lat - deg_offset,
        lon - deg_offset,
        lat + deg_offset,
        lon + deg_offset,
    );
    format!(
        "{} AND ST_Distance_Sphere(geom, ST_Point({}, {})) <= {}",
        bbox, lon, lat, radius_m
    )
}

/// SELECT expression for distance in km (haversine via ST_Distance_Sphere).
pub fn distance_select(lat: f64, lon: f64) -> String {
    format!(
        "ST_Distance_Sphere(geom, ST_Point({}, {})) / 1000.0 AS _distance_km",
        lon, lat
    )
}

/// WHERE fragment for point-in-polygon using ST_Contains (R-tree accelerated).
pub fn within_filter_geojson(geojson_str: &str) -> String {
    format!(
        "ST_Contains(ST_GeomFromGeoJSON('{}'), geom)",
        geojson_str.replace('\'', "''")
    )
}

// --- Query functions ---

/// Execute a query with optional spatial filter, where/select/group_by/agg/limit.
/// When `spatial_where` is provided, queries `raw_data` (has geom + R-tree index)
/// and excludes geom from output. Otherwise queries the `data` view.
#[allow(clippy::too_many_arguments)]
pub fn query(
    db: &Mutex<Connection>,
    spatial_where: Option<&str>,
    where_clauses: &[(String, String)],
    select_cols: Option<&[String]>,
    group_by: Option<&str>,
    agg: Option<&str>,
    limit: usize,
    extra_select: Option<&str>,
    order_by: Option<&str>,
) -> Result<Vec<Value>, AppError> {
    // Aggregation path
    if let (Some(gb), Some(a)) = (group_by, agg) {
        db::validate_column_name(gb)?;
        return query_aggregate(db, spatial_where, where_clauses, gb, a, limit);
    }

    let table = if spatial_where.is_some() {
        "raw_data"
    } else {
        "data"
    };

    let select = match select_cols {
        Some(cols) => {
            for c in cols {
                db::validate_column_name(c)?;
            }
            let mut s = cols.join(", ");
            if let Some(extra) = extra_select {
                s.push_str(", ");
                s.push_str(extra);
            }
            s
        }
        None => {
            let base = if spatial_where.is_some() {
                "* EXCLUDE (geom)"
            } else {
                "*"
            };
            let mut s = String::from(base);
            if let Some(extra) = extra_select {
                s.push_str(", ");
                s.push_str(extra);
            }
            s
        }
    };

    let mut sql = format!("SELECT {} FROM {}", select, table);
    let mut conditions = Vec::new();

    if let Some(sw) = spatial_where {
        conditions.push(sw.to_string());
    }

    for (col, val) in where_clauses {
        db::validate_column_name(col)?;
        conditions.push(format!(
            "CAST(\"{}\" AS VARCHAR) = '{}'",
            col,
            escape_sql_value(val)
        ));
    }

    if !conditions.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conditions.join(" AND "));
    }

    if let Some(ob) = order_by {
        sql.push_str(" ORDER BY ");
        sql.push_str(ob);
    }

    sql.push_str(&format!(" LIMIT {}", limit));

    execute_query_to_json(db, &sql)
}

fn query_aggregate(
    db: &Mutex<Connection>,
    spatial_where: Option<&str>,
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

    let table2 = if spatial_where.is_some() {
        "raw_data"
    } else {
        "data"
    };
    let mut sql = format!("SELECT \"{}\", {} FROM {}", group_by, agg_expr, table2);

    let mut conditions = Vec::new();

    if let Some(sw) = spatial_where {
        conditions.push(sw.to_string());
    }

    for (col, val) in where_clauses {
        db::validate_column_name(col)?;
        conditions.push(format!(
            "CAST(\"{}\" AS VARCHAR) = '{}'",
            col,
            escape_sql_value(val)
        ));
    }

    if !conditions.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conditions.join(" AND "));
    }

    sql.push_str(&format!(" GROUP BY \"{}\"", group_by));
    sql.push_str(&format!(" LIMIT {}", limit));

    execute_query_to_json(db, &sql)
}

/// Query lat/lon points within a bounding box (uses R-tree via raw_data).
pub fn query_points_in_bbox(
    db: &Mutex<Connection>,
    lat_col: &str,
    lon_col: &str,
    min_lat: f64,
    min_lon: f64,
    max_lat: f64,
    max_lon: f64,
) -> Result<Vec<(f64, f64)>, AppError> {
    db::validate_column_name(lat_col)?;
    db::validate_column_name(lon_col)?;
    let filter = bbox_filter(min_lat, min_lon, max_lat, max_lon);
    let sql = format!(
        "SELECT \"{}\", \"{}\" FROM raw_data WHERE {}",
        lat_col, lon_col, filter
    );
    let conn = db::lock_db(db)?;
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("SQL prepare error: {} — {}", e, sql)))?;
    let mut rows = stmt
        .query([])
        .map_err(|e| AppError::Internal(anyhow::anyhow!("SQL query error: {} — {}", e, sql)))?;
    let mut points = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Row fetch error: {}", e)))?
    {
        let lat: f64 = row
            .get(0)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("lat read error: {}", e)))?;
        let lon: f64 = row
            .get(1)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("lon read error: {}", e)))?;
        points.push((lat, lon));
    }
    Ok(points)
}

/// Query full rows within a bounding box (uses R-tree via raw_data).
pub fn query_rows_in_bbox(
    db: &Mutex<Connection>,
    min_lat: f64,
    min_lon: f64,
    max_lat: f64,
    max_lon: f64,
    limit: usize,
) -> Result<Vec<Value>, AppError> {
    let filter = bbox_filter(min_lat, min_lon, max_lat, max_lon);
    let sql = format!(
        "SELECT * EXCLUDE (geom) FROM raw_data WHERE {} LIMIT {}",
        filter, limit
    );
    execute_query_to_json(db, &sql)
}

/// Execute a SQL query and return results as Vec<serde_json::Value>.
pub fn execute_query_to_json(db: &Mutex<Connection>, sql: &str) -> Result<Vec<Value>, AppError> {
    let conn = db::lock_db(db)?;
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("SQL prepare error: {} — {}", e, sql)))?;

    let mut rows = stmt
        .query([])
        .map_err(|e| AppError::Internal(anyhow::anyhow!("SQL query error: {} — {}", e, sql)))?;

    let stmt_ref = rows
        .as_ref()
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("No statement backing query results")))?;
    let col_count = stmt_ref.column_count();
    let col_names: Vec<String> = (0..col_count)
        .map(|i| {
            stmt_ref
                .column_name(i)
                .map_or("?".to_string(), |v| v.to_string())
        })
        .collect();
    let col_types: Vec<DataType> = (0..col_count).map(|i| stmt_ref.column_type(i)).collect();

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
        DataType::Int8
        | DataType::Int16
        | DataType::Int32
        | DataType::Int64
        | DataType::UInt8
        | DataType::UInt16
        | DataType::UInt32 => match row.get::<_, i64>(idx) {
            Ok(v) => json!(v),
            Err(_) => Value::Null,
        },
        DataType::UInt64 => match row.get::<_, u64>(idx) {
            Ok(v) if v <= i64::MAX as u64 => json!(v),
            Ok(v) => json!(v.to_string()),
            Err(_) => Value::Null,
        },
        DataType::Float16
        | DataType::Float32
        | DataType::Float64
        | DataType::Decimal128(_, _)
        | DataType::Decimal256(_, _) => match row.get::<_, f64>(idx) {
            Ok(v) if v.is_nan() || v.is_infinite() => Value::Null,
            Ok(v) => json!(v),
            Err(_) => Value::Null,
        },
        DataType::Boolean => match row.get::<_, bool>(idx) {
            Ok(v) => json!(v),
            Err(_) => Value::Null,
        },
        DataType::Date32 => match row.get::<_, i32>(idx) {
            Ok(days) => {
                let date = NaiveDate::from_num_days_from_ce_opt(days + 719_163)
                    .map(|d| d.format("%Y-%m-%d").to_string());
                match date {
                    Some(s) => json!(s),
                    None => json!(days),
                }
            }
            Err(_) => Value::Null,
        },
        DataType::Date64 | DataType::Timestamp(_, _) => match row.get::<_, String>(idx) {
            Ok(v) => json!(v),
            Err(_) => match row.get::<_, i64>(idx) {
                Ok(v) => json!(v),
                Err(_) => Value::Null,
            },
        },
        _ => match row.get::<_, String>(idx) {
            Ok(v) => json!(v),
            Err(_) => Value::Null,
        },
    }
}

fn escape_sql_value(s: &str) -> String {
    s.replace('\'', "''")
}
