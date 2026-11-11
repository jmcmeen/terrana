//! SQL query builders and spatial filter helpers. Column names are quoted with
//! [`db::quote_ident`], so any name is read as one identifier (the HTTP handler first
//! checks them against the schema with [`db::resolve_column`]); string values are
//! escaped before interpolation into SQL.

use crate::db;
use crate::db::filter::Condition;
use crate::error::AppError;
use chrono::NaiveDate;
use duckdb::arrow::datatypes::DataType;
use duckdb::types::ValueRef;
use duckdb::Connection;
use serde_json::{json, Value};
use std::sync::Mutex;

/// Default row limit when the client does not specify `limit`.
pub const DEFAULT_LIMIT: usize = 1000;
/// Hard cap on the number of rows any query may return.
pub const MAX_RESULT_LIMIT: usize = 100_000;

// --- Spatial SQL helpers (DuckDB spatial extension) ---

/// WHERE fragment for rows with a point. A row whose coordinates are blank, not
/// numbers, NaN or infinite has a null `geom`; spatial queries leave it out.
pub const HAS_POINT: &str = "geom IS NOT NULL";

/// WHERE fragment for bounding box using ST_Intersects (R-tree accelerated).
pub fn bbox_filter(min_lat: f64, min_lon: f64, max_lat: f64, max_lon: f64) -> String {
    format!(
        "ST_Intersects(geom, ST_MakeEnvelope({}, {}, {}, {}))",
        min_lon, min_lat, max_lon, max_lat
    )
}

/// WHERE fragment for a bounding box as plain coordinate ranges, for queries that
/// sort the whole match (paged `/query` orders by `rowid`). Every geometry here is
/// a point, so it selects the same rows as [`bbox_filter`], edges included. But
/// where the R-tree path then evaluates `ST_Intersects` on every matching row
/// before the sort (2 s for a box over 1M rows), ranges are a vectorized compare
/// (6 ms).
pub fn bbox_point_filter(min_lat: f64, min_lon: f64, max_lat: f64, max_lon: f64) -> String {
    format!(
        "ST_X(geom) BETWEEN {} AND {} AND ST_Y(geom) BETWEEN {} AND {}",
        min_lon, max_lon, min_lat, max_lat
    )
}

/// WHERE fragment for radius: an R-tree bounding box to narrow the search, then the
/// exact `ST_Distance_Sphere` test.
///
/// A degree of longitude shrinks with latitude, so the box's longitude half-width is
/// widened by 1/cos of the farthest latitude the circle reaches. Where the box can't
/// bound the circle (it would reach a pole, or cross the ±180° meridian), there is
/// no box, only the distance test.
pub fn radius_filter(lat: f64, lon: f64, radius_m: f64) -> String {
    let distance = format!(
        "ST_Distance_Sphere(geom, ST_Point({}, {})) <= {}",
        lon, lat, radius_m
    );
    match radius_box(lat, lon, radius_m) {
        Some((min_lat, min_lon, max_lat, max_lon)) => format!(
            "{} AND {}",
            bbox_filter(min_lat, min_lon, max_lat, max_lon),
            distance
        ),
        None => distance,
    }
}

/// The (min_lat, min_lon, max_lat, max_lon) box around a circle of `radius_m`, with a
/// 1.5× margin, or `None` where a latitude/longitude box can't bound it.
fn radius_box(lat: f64, lon: f64, radius_m: f64) -> Option<(f64, f64, f64, f64)> {
    let dlat = radius_m / 111_000.0 * 1.5;
    let far_lat = lat.abs() + dlat;
    if far_lat >= 89.0 {
        return None;
    }
    let dlon = dlat / far_lat.to_radians().cos();
    if lon - dlon < -180.0 || lon + dlon > 180.0 {
        return None;
    }
    Some((lat - dlat, lon - dlon, lat + dlat, lon + dlon))
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
/// and excludes geom from output. Otherwise queries the `data` view. The first page
/// of [`query_page`].
#[allow(clippy::too_many_arguments)]
pub fn query(
    db: &Mutex<Connection>,
    spatial_where: Option<&str>,
    conditions: &[Condition],
    select_cols: Option<&[String]>,
    group_by: Option<&str>,
    agg: Option<&str>,
    limit: usize,
    extra_select: Option<&str>,
    order_by: Option<&str>,
) -> Result<Vec<Value>, AppError> {
    run_query(
        db,
        spatial_where,
        conditions,
        select_cols,
        group_by,
        agg,
        limit,
        None,
        extra_select,
        order_by,
    )
}

/// Like [`query`], skipping the first `offset` rows of a fully ordered result (by
/// `order_by`, then the loader's `rowid`; aggregates by the group column), so
/// consecutive pages neither overlap nor skip rows. [`query`] skips that ordering
/// when it isn't asked for, which spares callers that never page a sort.
#[allow(clippy::too_many_arguments)]
pub fn query_page(
    db: &Mutex<Connection>,
    spatial_where: Option<&str>,
    conditions: &[Condition],
    select_cols: Option<&[String]>,
    group_by: Option<&str>,
    agg: Option<&str>,
    limit: usize,
    offset: usize,
    extra_select: Option<&str>,
    order_by: Option<&str>,
) -> Result<Vec<Value>, AppError> {
    run_query(
        db,
        spatial_where,
        conditions,
        select_cols,
        group_by,
        agg,
        limit,
        Some(offset),
        extra_select,
        order_by,
    )
}

/// Shared body of [`query`] and [`query_page`]. `offset: Some(_)` asks for a
/// stable total order and an OFFSET; `None` keeps the caller's `order_by` only.
#[allow(clippy::too_many_arguments)]
fn run_query(
    db: &Mutex<Connection>,
    spatial_where: Option<&str>,
    conditions: &[Condition],
    select_cols: Option<&[String]>,
    group_by: Option<&str>,
    agg: Option<&str>,
    limit: usize,
    offset: Option<usize>,
    extra_select: Option<&str>,
    order_by: Option<&str>,
) -> Result<Vec<Value>, AppError> {
    // Aggregation path
    if let (Some(gb), Some(a)) = (group_by, agg) {
        return query_aggregate(db, spatial_where, conditions, gb, a, limit, offset);
    }

    let table = if spatial_where.is_some() {
        "raw_data"
    } else {
        "data"
    };

    let select = match select_cols {
        Some(cols) => {
            let mut s = cols
                .iter()
                .map(|c| db::quote_ident(c))
                .collect::<Vec<_>>()
                .join(", ");
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
    sql.push_str(&where_clause(spatial_where, conditions));

    match offset {
        Some(off) => {
            sql.push_str(&order_clause(order_by));
            sql.push_str(&format!(" LIMIT {} OFFSET {}", limit, off));
        }
        None => {
            if let Some(ob) = order_by {
                sql.push_str(" ORDER BY ");
                sql.push_str(ob);
            }
            sql.push_str(&format!(" LIMIT {}", limit));
        }
    }

    execute_query_to_json(db, &sql)
}

/// ` WHERE ...` joining the spatial filter and the `where` conditions with AND,
/// or nothing when there are none.
fn where_clause(spatial_where: Option<&str>, conditions: &[Condition]) -> String {
    let parts: Vec<&str> = spatial_where
        .into_iter()
        .chain(conditions.iter().map(Condition::sql))
        .collect();
    if parts.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", parts.join(" AND "))
    }
}

/// ORDER BY clause ending in the loader's `rowid`, so ties (and unordered queries)
/// come back in the same order on every request.
fn order_clause(order_by: Option<&str>) -> String {
    match order_by {
        Some(ob) => format!(" ORDER BY {}, rowid", ob),
        None => " ORDER BY rowid".to_string(),
    }
}

/// An `agg` value as (SQL function, column): `count`, or `sum:col`, `avg:col`,
/// `min:col` or `max:col`, the function matched whatever its case; spaces around
/// either part are dropped.
pub fn parse_agg(agg: &str) -> Result<(&'static str, Option<&str>), AppError> {
    let agg = agg.trim();
    if agg.eq_ignore_ascii_case("count") {
        return Ok(("COUNT", None));
    }
    let func = agg.split_once(':').and_then(|(name, col)| {
        ["SUM", "AVG", "MIN", "MAX"]
            .into_iter()
            .find(|f| f.eq_ignore_ascii_case(name.trim()))
            .zip(Some(col.trim()).filter(|c| !c.is_empty()))
    });
    match func {
        Some((func, col)) => Ok((func, Some(col))),
        None => Err(AppError::BadRequest(format!(
            "Invalid agg '{}'. Use count, sum:col, avg:col, min:col or max:col",
            agg
        ))),
    }
}

fn query_aggregate(
    db: &Mutex<Connection>,
    spatial_where: Option<&str>,
    conditions: &[Condition],
    group_by: &str,
    agg: &str,
    limit: usize,
    offset: Option<usize>,
) -> Result<Vec<Value>, AppError> {
    let (func, col) = parse_agg(agg)?;
    let (arg, mut alias) = match col {
        Some(col) => (
            db::quote_ident(col),
            format!("{}_{}", func.to_lowercase(), col),
        ),
        None => ("*".to_string(), "count".to_string()),
    };
    // A row can't hold two keys of one name: grouping by a column called `count` (or
    // `sum_x` with `sum:x`) would lose the group value, so the aggregate takes a `_`.
    if alias.eq_ignore_ascii_case(group_by) {
        alias.insert(0, '_');
    }
    let agg_expr = format!("{}({}) AS {}", func, arg, db::quote_ident(&alias));
    let gb = db::quote_ident(group_by);

    let table2 = if spatial_where.is_some() {
        "raw_data"
    } else {
        "data"
    };
    let mut sql = format!("SELECT {}, {} FROM {}", gb, agg_expr, table2);

    sql.push_str(&where_clause(spatial_where, conditions));

    sql.push_str(&format!(" GROUP BY {}", gb));
    match offset {
        Some(off) => {
            // Groups are unique, so ordering by the group column alone is stable.
            sql.push_str(&format!(" ORDER BY {}", gb));
            sql.push_str(&format!(" LIMIT {} OFFSET {}", limit, off));
        }
        None => sql.push_str(&format!(" LIMIT {}", limit)),
    }

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
    let filter = bbox_filter(min_lat, min_lon, max_lat, max_lon);
    let sql = format!(
        "SELECT {}, {} FROM raw_data WHERE {}",
        db::quote_ident(lat_col),
        db::quote_ident(lon_col),
        filter
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

/// A whole number as a JSON number when it fits in an `i64`, else its exact digits as
/// a string: past that, many JSON readers (JavaScript's among them) would round it.
fn int_json(v: i128) -> Value {
    match i64::try_from(v) {
        Ok(v) => json!(v),
        Err(_) => json!(v.to_string()),
    }
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
            Ok(v) => int_json(i128::from(v)),
            Err(_) => Value::Null,
        },
        // HUGEINT (DuckDB's SUM of integers), UHUGEINT and DECIMAL(p, 0) all arrive as
        // Decimal128(38, 0); DuckDB's own value type tells them apart.
        DataType::Decimal128(_, 0) => match row.get_ref(idx) {
            Ok(ValueRef::HugeInt(v)) => int_json(v),
            Ok(ValueRef::UHugeInt(v)) => i128::try_from(v)
                .map(int_json)
                .unwrap_or_else(|_| json!(v.to_string())),
            Ok(ValueRef::Decimal(d)) if d.scale() == 0 => int_json(d.value()),
            Ok(ValueRef::Null) | Err(_) => Value::Null,
            Ok(_) => match row.get::<_, f64>(idx) {
                Ok(v) if v.is_finite() => json!(v),
                _ => Value::Null,
            },
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bbox_filter_uses_lon_lat_order() {
        // ST_MakeEnvelope takes (min_lon, min_lat, max_lon, max_lat).
        let f = bbox_filter(35.0, -84.0, 37.0, -81.0);
        assert!(f.contains("ST_MakeEnvelope(-84, 35, -81, 37)"));
        assert!(f.starts_with("ST_Intersects(geom,"));
    }

    #[test]
    fn bbox_point_filter_uses_x_for_lon_and_y_for_lat() {
        assert_eq!(
            bbox_point_filter(35.0, -84.0, 37.0, -81.0),
            "ST_X(geom) BETWEEN -84 AND -81 AND ST_Y(geom) BETWEEN 35 AND 37"
        );
    }

    #[test]
    fn parse_agg_takes_the_supported_forms_only() {
        assert_eq!(parse_agg(" Count ").unwrap(), ("COUNT", None));
        assert_eq!(parse_agg("Sum: n ").unwrap(), ("SUM", Some("n")));
        for bad in ["median:n", "sum", "sum:", "count:n", ""] {
            let err = parse_agg(bad).unwrap_err().to_string();
            assert!(err.contains("Use count, sum:col"), "{bad}: {err}");
        }
    }

    #[test]
    fn whole_numbers_are_integers_and_text_past_i64() {
        let db = Mutex::new(db::create_connection().unwrap());
        let rows = execute_query_to_json(
            &db,
            "SELECT 6::HUGEINT AS h, (-9223372036854775808)::HUGEINT - 1 AS h_low, \
             NULL::HUGEINT AS h_null, 340282366920938463463374607431768211455::UHUGEINT AS uh, \
             12::DECIMAL(10, 0) AS d0, 1.25::DECIMAL(10, 2) AS d2, \
             18446744073709551615::UBIGINT AS ub",
        )
        .unwrap();
        assert_eq!(
            rows[0],
            json!({
                "h": 6,
                "h_low": "-9223372036854775809",
                "h_null": null,
                "uh": "340282366920938463463374607431768211455",
                "d0": 12,
                "d2": 1.25,
                "ub": "18446744073709551615",
            })
        );
    }

    #[test]
    fn order_clause_ends_with_rowid() {
        assert_eq!(order_clause(None), " ORDER BY rowid");
        assert_eq!(
            order_clause(Some("_distance_km ASC")),
            " ORDER BY _distance_km ASC, rowid"
        );
    }

    #[test]
    fn radius_box_widens_longitude_with_latitude() {
        // At the equator the box is square.
        let (_, min_lon, _, max_lon) = radius_box(0.0, 10.0, 10_000.0).unwrap();
        assert!((max_lon - min_lon - 2.0 * 10_000.0 / 111_000.0 * 1.5).abs() < 1e-3);
        // At 60°N a degree of longitude is about 55.6 km: the box must reach past 10 km east.
        let (_, _, _, max_lon) = radius_box(60.0, 10.0, 10_000.0).unwrap();
        let east_km = (max_lon - 10.0) * 111.32 * 60f64.to_radians().cos();
        assert!(east_km > 10.0, "box reaches only {east_km} km east");
    }

    #[test]
    fn no_box_near_a_pole_or_across_the_antimeridian() {
        assert!(radius_box(88.9, 0.0, 50_000.0).is_none());
        assert!(radius_box(10.0, 179.95, 10_000.0).is_none());
        assert!(radius_box(10.0, -179.95, 10_000.0).is_none());
        assert!(!radius_filter(10.0, 179.95, 10_000.0).contains("ST_Intersects"));
        assert!(radius_filter(36.5, -82.5, 10_000.0).contains("ST_Intersects"));
    }

    #[test]
    fn distance_select_aliases_km() {
        let s = distance_select(36.5, -82.5);
        assert!(s.contains("ST_Distance_Sphere(geom, ST_Point(-82.5, 36.5))"));
        assert!(s.ends_with("AS _distance_km"));
    }

    #[test]
    fn within_filter_escapes_geojson() {
        let f = within_filter_geojson("{\"x\":\"o'brien\"}");
        assert!(f.contains("o''brien"));
        assert!(f.starts_with("ST_Contains(ST_GeomFromGeoJSON("));
    }
}
