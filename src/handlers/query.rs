//! `GET /query` — radius, bounding-box, and nearest-neighbor spatial queries,
//! plus the shared `select`/`where`/`group_by`/`agg`/`limit`/`format` parameters.

use crate::db;
use crate::db::query::{DEFAULT_LIMIT, MAX_RESULT_LIMIT};
use crate::error::AppError;
use crate::output;
use crate::server::AppState;
use axum::extract::{Query, State};
use axum::response::Response;
use serde::Deserialize;
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

/// Dispatch a `GET /query` request to the bbox, nearest, radius, or plain-table path
/// based on which parameters are present, then serialize to the requested format.
pub async fn query(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let qp = parse_query_params(&params)?;
    let limit = qp.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_RESULT_LIMIT);
    let format = qp.format.as_deref().unwrap_or("json");
    let where_clauses = parse_where_clauses(qp.where_filter.as_deref());
    let select_cols = parse_select(qp.select.as_deref());

    if let Some(bbox_str) = &qp.bbox {
        // Bounding box query
        let bbox = parse_bbox(bbox_str)?;
        let spatial = db::query::bbox_filter(bbox.0, bbox.1, bbox.2, bbox.3);

        let rows = db::query::query(
            &state.db,
            Some(&spatial),
            &where_clauses,
            select_cols.as_deref(),
            qp.group_by.as_deref(),
            qp.agg.as_deref(),
            limit,
            None,
            None,
        )?;
        output::format_response(&rows, format, &state)
    } else if let (Some(lat), Some(lon), Some(nearest)) = (qp.lat, qp.lon, qp.nearest) {
        // Nearest neighbor query — ORDER BY distance + LIMIT
        let extra = db::query::distance_select(lat, lon);
        let rows = db::query::query(
            &state.db,
            Some("geom IS NOT NULL"),
            &where_clauses,
            select_cols.as_deref(),
            qp.group_by.as_deref(),
            qp.agg.as_deref(),
            nearest,
            Some(&extra),
            Some("_distance_km ASC"),
        )?;
        output::format_response(&rows, format, &state)
    } else if let (Some(lat), Some(lon), Some(radius_str)) = (qp.lat, qp.lon, &qp.radius) {
        // Radius query
        let radius_m = parse_radius(radius_str)?;
        let spatial = db::query::radius_filter(lat, lon, radius_m);
        let extra = db::query::distance_select(lat, lon);

        let rows = db::query::query(
            &state.db,
            Some(&spatial),
            &where_clauses,
            select_cols.as_deref(),
            qp.group_by.as_deref(),
            qp.agg.as_deref(),
            limit,
            Some(&extra),
            Some("_distance_km ASC"),
        )?;
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
            None,
            None,
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
    let (min_lat, min_lon, max_lat, max_lon) = (parts[0], parts[1], parts[2], parts[3]);
    if !(-90.0..=90.0).contains(&min_lat) || !(-90.0..=90.0).contains(&max_lat) {
        return Err(AppError::BadRequest(
            "bbox latitudes must be within [-90, 90]".into(),
        ));
    }
    if !(-180.0..=180.0).contains(&min_lon) || !(-180.0..=180.0).contains(&max_lon) {
        return Err(AppError::BadRequest(
            "bbox longitudes must be within [-180, 180]".into(),
        ));
    }
    if min_lat > max_lat || min_lon > max_lon {
        return Err(AppError::BadRequest(
            "bbox requires minlat<=maxlat and minlon<=maxlon".into(),
        ));
    }
    Ok((min_lat, min_lon, max_lat, max_lon))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bbox_parses_valid_input() {
        assert_eq!(
            parse_bbox("35.0,-84.0,37.0,-81.0").unwrap(),
            (35.0, -84.0, 37.0, -81.0)
        );
    }

    #[test]
    fn bbox_rejects_out_of_range_and_inverted() {
        assert!(parse_bbox("35.0,-84.0,37.0").is_err()); // too few
        assert!(parse_bbox("-100.0,0.0,10.0,1.0").is_err()); // lat < -90
        assert!(parse_bbox("0.0,-200.0,1.0,1.0").is_err()); // lon < -180
        assert!(parse_bbox("37.0,-81.0,35.0,-84.0").is_err()); // min > max
        assert!(parse_bbox("a,b,c,d").is_err()); // non-numeric
    }

    #[test]
    fn radius_parses_units() {
        assert_eq!(parse_radius("10km").unwrap(), 10_000.0);
        assert_eq!(parse_radius("5000m").unwrap(), 5000.0);
        assert_eq!(parse_radius("1mi").unwrap(), 1609.344);
        assert_eq!(parse_radius("100").unwrap(), 100.0);
        assert!(parse_radius("abc").is_err());
    }

    #[test]
    fn where_clauses_split_on_colon() {
        let clauses = parse_where_clauses(Some("quality_grade:research,species:Bombus"));
        assert_eq!(
            clauses,
            vec![
                ("quality_grade".to_string(), "research".to_string()),
                ("species".to_string(), "Bombus".to_string()),
            ]
        );
        assert!(parse_where_clauses(None).is_empty());
    }

    #[test]
    fn select_trims_columns() {
        assert_eq!(
            parse_select(Some("species, observed_on")),
            Some(vec!["species".to_string(), "observed_on".to_string()])
        );
        assert_eq!(parse_select(None), None);
    }
}
