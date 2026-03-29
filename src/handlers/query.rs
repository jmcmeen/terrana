use crate::db;
use crate::error::AppError;
use crate::output;
use crate::server::AppState;
use axum::extract::{Query, State};
use axum::response::Response;
use geo::{Distance, Geodesic};
use geo_types::Point;
use rstar::AABB;
use serde::Deserialize;
use std::cmp::Ordering;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub struct QueryParams {
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub radius: Option<String>,
    pub bbox: Option<String>,
    pub nearest: Option<usize>,
    pub select: Option<String>,
    #[serde(rename = "where")]
    pub where_filter: Option<String>,
    pub group_by: Option<String>,
    pub agg: Option<String>,
    pub limit: Option<usize>,
    pub format: Option<String>,
}

pub async fn query(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let qp = parse_query_params(&params)?;
    let limit = qp.limit.unwrap_or(1000).min(100_000);
    let format = qp.format.as_deref().unwrap_or("json");
    let where_clauses = parse_where_clauses(qp.where_filter.as_deref());
    let select_cols = parse_select(qp.select.as_deref());

    // Validate column names
    for (col, _) in &where_clauses {
        db::validate_column_name(col)?;
    }
    if let Some(ref cols) = select_cols {
        for col in cols {
            db::validate_column_name(col)?;
        }
    }
    if let Some(ref gb) = qp.group_by {
        db::validate_column_name(gb)?;
    }

    if let Some(bbox_str) = &qp.bbox {
        // Bounding box query
        let bbox = parse_bbox(bbox_str)?;
        let envelope = AABB::from_corners(
            [bbox.1, bbox.0], // [min_lon, min_lat]
            [bbox.3, bbox.2], // [max_lon, max_lat]
        );
        let candidates: Vec<i64> = state
            .index
            .locate_in_envelope(&envelope)
            .map(|p| p.rowid)
            .collect();

        let rows = db::query::query(
            &state.db,
            Some(&candidates),
            &where_clauses,
            select_cols.as_deref(),
            qp.group_by.as_deref(),
            qp.agg.as_deref(),
            limit,
        )?;
        output::format_response(&rows, format, &state)
    } else if let (Some(lat), Some(lon), Some(nearest)) = (qp.lat, qp.lon, qp.nearest) {
        // Nearest neighbor query
        let origin = Point::new(lon, lat);
        let mut results: Vec<(i64, f64)> = state
            .index
            .nearest_neighbor_iter(&[lon, lat])
            .take(nearest)
            .map(|p| {
                let dist_m = Geodesic::distance(origin, Point::new(p.lon, p.lat));
                (p.rowid, dist_m / 1000.0)
            })
            .collect();
        results.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal));

        let rowids: Vec<i64> = results.iter().map(|r| r.0).collect();
        let distances: HashMap<i64, f64> = results.into_iter().collect();
        let mut rows = db::query::query(
            &state.db,
            Some(&rowids),
            &where_clauses,
            select_cols.as_deref(),
            qp.group_by.as_deref(),
            qp.agg.as_deref(),
            limit,
        )?;
        inject_distances(&mut rows, &distances);
        rows.sort_by(|a, b| {
            let da = a
                .get("_distance_km")
                .and_then(|v| v.as_f64())
                .unwrap_or(f64::MAX);
            let db = b
                .get("_distance_km")
                .and_then(|v| v.as_f64())
                .unwrap_or(f64::MAX);
            da.partial_cmp(&db).unwrap_or(Ordering::Equal)
        });
        output::format_response(&rows, format, &state)
    } else if let (Some(lat), Some(lon), Some(radius_str)) = (qp.lat, qp.lon, &qp.radius) {
        // Radius query
        let radius_m = parse_radius(radius_str)?;
        let origin = Point::new(lon, lat);

        let deg_offset = radius_m / 111_000.0 * 1.5;
        let envelope = AABB::from_corners(
            [lon - deg_offset, lat - deg_offset],
            [lon + deg_offset, lat + deg_offset],
        );

        let mut results: Vec<(i64, f64)> = state
            .index
            .locate_in_envelope(&envelope)
            .filter_map(|p| {
                let dist_m = Geodesic::distance(origin, Point::new(p.lon, p.lat));
                if dist_m <= radius_m {
                    Some((p.rowid, dist_m / 1000.0))
                } else {
                    None
                }
            })
            .collect();
        results.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal));

        let rowids: Vec<i64> = results.iter().map(|r| r.0).collect();
        let distances: HashMap<i64, f64> = results.into_iter().collect();
        let mut rows = db::query::query(
            &state.db,
            Some(&rowids),
            &where_clauses,
            select_cols.as_deref(),
            qp.group_by.as_deref(),
            qp.agg.as_deref(),
            limit,
        )?;
        inject_distances(&mut rows, &distances);
        rows.sort_by(|a, b| {
            let da = a
                .get("_distance_km")
                .and_then(|v| v.as_f64())
                .unwrap_or(f64::MAX);
            let db = b
                .get("_distance_km")
                .and_then(|v| v.as_f64())
                .unwrap_or(f64::MAX);
            da.partial_cmp(&db).unwrap_or(Ordering::Equal)
        });
        output::format_response(&rows, format, &state)
    } else {
        // No spatial filter — plain table query
        let rows = db::query::query(
            &state.db,
            None,
            &where_clauses,
            select_cols.as_deref(),
            qp.group_by.as_deref(),
            qp.agg.as_deref(),
            limit,
        )?;
        output::format_response(&rows, format, &state)
    }
}

fn parse_query_params(params: &HashMap<String, String>) -> Result<QueryParams, AppError> {
    Ok(QueryParams {
        lat: params.get("lat").and_then(|v| v.parse().ok()),
        lon: params.get("lon").and_then(|v| v.parse().ok()),
        radius: params.get("radius").cloned(),
        bbox: params.get("bbox").cloned(),
        nearest: params.get("nearest").and_then(|v| v.parse().ok()),
        select: params.get("select").cloned(),
        where_filter: params.get("where").cloned(),
        group_by: params.get("group_by").cloned(),
        agg: params.get("agg").cloned(),
        limit: params.get("limit").and_then(|v| v.parse().ok()),
        format: params.get("format").cloned(),
    })
}

fn parse_bbox(s: &str) -> Result<(f64, f64, f64, f64), AppError> {
    let parts: Vec<f64> = s
        .split(',')
        .map(|p| p.trim().parse::<f64>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| {
            AppError::BadRequest(
                "Invalid bbox format. Expected: minlat,minlon,maxlat,maxlon".into(),
            )
        })?;
    if parts.len() != 4 {
        return Err(AppError::BadRequest(
            "bbox requires exactly 4 values: minlat,minlon,maxlat,maxlon".into(),
        ));
    }
    Ok((parts[0], parts[1], parts[2], parts[3]))
}

fn parse_radius(s: &str) -> Result<f64, AppError> {
    let s = s.trim();
    if let Some(val) = s.strip_suffix("km") {
        val.trim().parse::<f64>().map(|v| v * 1000.0)
    } else if let Some(val) = s.strip_suffix("mi") {
        val.trim().parse::<f64>().map(|v| v * 1609.344)
    } else if let Some(val) = s.strip_suffix("ft") {
        val.trim().parse::<f64>().map(|v| v * 0.3048)
    } else if let Some(val) = s.strip_suffix('m') {
        val.trim().parse::<f64>()
    } else {
        s.parse::<f64>()
    }
    .map_err(|_| {
        AppError::BadRequest(format!(
            "Invalid radius: '{}'. Use e.g. 10km, 5000m, 3mi",
            s
        ))
    })
}

fn parse_where_clauses(filter: Option<&str>) -> Vec<(String, String)> {
    let Some(f) = filter else { return vec![] };
    f.split(',')
        .filter_map(|clause| {
            let parts: Vec<&str> = clause.splitn(2, ':').collect();
            if parts.len() == 2 {
                Some((parts[0].to_string(), parts[1].to_string()))
            } else {
                None
            }
        })
        .collect()
}

fn parse_select(select: Option<&str>) -> Option<Vec<String>> {
    select.map(|s| s.split(',').map(|c| c.trim().to_string()).collect())
}

fn inject_distances(rows: &mut [serde_json::Value], distances: &HashMap<i64, f64>) {
    for row in rows.iter_mut() {
        if let Some(obj) = row.as_object_mut() {
            if let Some(rowid) = obj.get("rowid").and_then(|v| v.as_i64()) {
                if let Some(&dist) = distances.get(&rowid) {
                    obj.insert("_distance_km".to_string(), serde_json::json!(dist));
                }
            }
        }
    }
}
