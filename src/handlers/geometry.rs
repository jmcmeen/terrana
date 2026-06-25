//! `POST /geometry/*` endpoints — area, convex-hull, centroid, buffer, dissolve,
//! simplify, distance, and bounds.
//!
//! These handlers are thin Axum glue: they parse the JSON body into geo-types,
//! call the relevant pure function in [`crate::geometry`], and shape the JSON
//! response. All geodesic math lives in `crate::geometry`, never here.

use crate::db;
use crate::db::query::MAX_RESULT_LIMIT;
use crate::error::AppError;
use crate::geometry;
use crate::server::AppState;
use axum::extract::State;
use axum::Json;
use geo_types::{Geometry, Point, Polygon};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::convert::TryInto;

pub async fn area(
    State(_state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let polygons = extract_polygons_from_body(&body)?;
    let result = geometry::area::compute_area(&polygons);
    Ok(Json(to_value(&result)?))
}

pub async fn convex_hull(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let points = if let Some(query) = body.get("query") {
        let (min_lat, min_lon, max_lat, max_lon) = parse_bbox(query)?;
        let snap = state.snapshot();
        let raw_pts = db::query::query_points_in_bbox(
            &state.db,
            &snap.schema.lat_col,
            &snap.schema.lon_col,
            min_lat,
            min_lon,
            max_lat,
            max_lon,
        )?;
        raw_pts
            .into_iter()
            .map(|(lat, lon)| Point::new(lon, lat))
            .collect()
    } else {
        extract_points_from_body(&body)?
    };

    if points.len() < 3 {
        return Err(AppError::BadRequest(
            "Need at least 3 points for convex hull".into(),
        ));
    }

    let result = geometry::hull::compute_convex_hull(&points);
    let hull_geojson: geojson::Geometry = (&result.hull).into();

    Ok(Json(json!({
        "type": "Feature",
        "geometry": serde_json::to_value(&hull_geojson).unwrap_or(json!(null)),
        "properties": {
            "area_m2": result.area_m2,
            "area_km2": result.area_km2,
            "area_ha": result.area_ha,
            "perimeter_m": result.perimeter_m,
            "point_count": result.point_count,
        }
    })))
}

pub async fn centroid(
    State(_state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let geo_geom = single_geometry_from_body(&body)?;
    let c = geometry::measure::centroid(&geo_geom)
        .ok_or_else(|| AppError::Geometry("Could not compute centroid".into()))?;

    Ok(Json(json!({
        "centroid": {
            "type": "Point",
            "coordinates": [c.x(), c.y()]
        }
    })))
}

pub async fn buffer(
    State(_state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let geom_val = body
        .get("geometry")
        .ok_or_else(|| AppError::BadRequest("Missing 'geometry' field".into()))?;
    let distance = body
        .get("distance")
        .and_then(|v| v.as_f64())
        .ok_or_else(|| AppError::BadRequest("Missing 'distance' field".into()))?;
    let unit = body.get("unit").and_then(|v| v.as_str()).unwrap_or("m");
    let segments = body.get("segments").and_then(|v| v.as_u64()).unwrap_or(64) as usize;

    let distance_m = match unit {
        "km" => distance * 1000.0,
        "mi" => distance * 1609.344,
        "ft" => distance * 0.3048,
        _ => distance,
    };

    let geo_geom = geo_geometry_from_value(geom_val)?;
    let center = geometry::measure::centroid(&geo_geom)
        .ok_or_else(|| AppError::Geometry("Cannot compute centroid for buffer".into()))?;

    let poly = geometry::buffer::compute_buffer(center, distance_m, segments);
    let area = geometry::area::compute_area(std::slice::from_ref(&poly));
    let poly_geojson: geojson::Geometry = (&poly).into();

    Ok(Json(json!({
        "type": "Feature",
        "geometry": serde_json::to_value(&poly_geojson).unwrap_or(json!(null)),
        "properties": {
            "area_m2": area.area_m2,
            "area_km2": area.area_km2,
            "distance_m": distance_m,
            "segments": segments,
        }
    })))
}

pub async fn dissolve(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let by = body
        .get("by")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("Missing 'by' field".into()))?;
    let include_area = body
        .get("include_area")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let include_count = body
        .get("include_count")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let rows = if let Some(query) = body.get("query") {
        let (min_lat, min_lon, max_lat, max_lon) = parse_bbox(query)?;
        db::query::query_rows_in_bbox(
            &state.db,
            min_lat,
            min_lon,
            max_lat,
            max_lon,
            MAX_RESULT_LIMIT,
        )?
    } else {
        db::query::query(
            &state.db,
            None,
            &[],
            None,
            None,
            None,
            MAX_RESULT_LIMIT,
            None,
            None,
        )?
    };

    let snap = state.snapshot();
    let lat_col = &snap.schema.lat_col;
    let lon_col = &snap.schema.lon_col;
    let mut groups: HashMap<String, Vec<Point<f64>>> = HashMap::new();

    for row in &rows {
        let key = row
            .get(by)
            .map(|v| match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .unwrap_or_else(|| "null".to_string());

        let lat = row.get(lat_col).and_then(|v| v.as_f64());
        let lon = row.get(lon_col).and_then(|v| v.as_f64());

        if let (Some(lat), Some(lon)) = (lat, lon) {
            groups.entry(key).or_default().push(Point::new(lon, lat));
        }
    }

    let features = geometry::dissolve::dissolve_by(&groups, by, include_area, include_count);

    Ok(Json(json!({
        "type": "FeatureCollection",
        "features": features,
    })))
}

pub async fn simplify(
    State(_state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let geom_val = body
        .get("geometry")
        .ok_or_else(|| AppError::BadRequest("Missing 'geometry' field".into()))?;
    let tolerance = body
        .get("tolerance")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.001);
    let preserve_topology = body
        .get("preserve_topology")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let geo_geom = geo_geometry_from_value(geom_val)?;
    let simplified = geometry::simplify::simplify_geometry(geo_geom, tolerance, preserve_topology);
    let result_geojson: geojson::Geometry = (&simplified).into();

    Ok(Json(json!({
        "geometry": serde_json::to_value(&result_geojson).unwrap_or(json!(null)),
        "tolerance": tolerance,
        "preserve_topology": preserve_topology,
    })))
}

pub async fn distance(
    State(_state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let from_val = body
        .get("from")
        .ok_or_else(|| AppError::BadRequest("Missing 'from' field".into()))?;
    let to_val = body
        .get("to")
        .ok_or_else(|| AppError::BadRequest("Missing 'to' field".into()))?;

    let from_pt = extract_point(from_val, "from")?;
    let to_pt = extract_point(to_val, "to")?;

    let result = geometry::measure::geodesic_distance(from_pt, to_pt);
    Ok(Json(to_value(&result)?))
}

pub async fn bounds(
    State(_state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let geo_geom = single_geometry_from_body(&body)?;
    let result = geometry::measure::bounding_box(&geo_geom)
        .ok_or_else(|| AppError::Geometry("Could not compute bounding rect".into()))?;
    let envelope_geojson: geojson::Geometry = (&result.envelope).into();

    Ok(Json(json!({
        "bbox": result.bbox,
        "envelope": serde_json::to_value(&envelope_geojson).unwrap_or(json!(null)),
        "width_km": result.width_km,
        "height_km": result.height_km,
        "area_km2": result.area_km2,
    })))
}

// --- Helpers ---

/// Serialize a pure result struct into a JSON value, mapping failures to a 500.
fn to_value<T: serde::Serialize>(value: &T) -> Result<Value, AppError> {
    serde_json::to_value(value).map_err(|e| AppError::Internal(anyhow::anyhow!(e)))
}

/// Parse a `{ "bbox": [min_lat, min_lon, max_lat, max_lon] }` query object.
fn parse_bbox(query: &Value) -> Result<(f64, f64, f64, f64), AppError> {
    let bbox = query
        .get("bbox")
        .and_then(|v| v.as_array())
        .ok_or_else(|| AppError::BadRequest("Expected query.bbox array".into()))?;
    if bbox.len() != 4 {
        return Err(AppError::BadRequest("bbox must have 4 values".into()));
    }
    let mut coords = [0.0_f64; 4];
    for (i, coord) in coords.iter_mut().enumerate() {
        *coord = bbox[i]
            .as_f64()
            .ok_or_else(|| AppError::BadRequest("bbox values must be numeric".into()))?;
    }
    Ok((coords[0], coords[1], coords[2], coords[3]))
}

/// Parse a body that is a single GeoJSON geometry or a Feature wrapping one.
fn single_geometry_from_body(body: &Value) -> Result<Geometry<f64>, AppError> {
    let geojson: geojson::GeoJson = body
        .to_string()
        .parse()
        .map_err(|e| AppError::BadRequest(format!("Invalid GeoJSON: {}", e)))?;

    let geom = match geojson {
        geojson::GeoJson::Geometry(g) => g,
        geojson::GeoJson::Feature(f) => f
            .geometry
            .ok_or_else(|| AppError::BadRequest("Feature has no geometry".into()))?,
        _ => return Err(AppError::BadRequest("Expected a single geometry".into())),
    };

    geom.try_into()
        .map_err(|e| AppError::Geometry(format!("{}", e)))
}

/// Parse a single GeoJSON geometry from a JSON value.
fn geo_geometry_from_value(val: &Value) -> Result<Geometry<f64>, AppError> {
    let geojson: geojson::Geometry = serde_json::from_value(val.clone())
        .map_err(|e| AppError::BadRequest(format!("Invalid geometry: {}", e)))?;
    geojson
        .try_into()
        .map_err(|e| AppError::Geometry(format!("{}", e)))
}

fn extract_polygons_from_body(body: &Value) -> Result<Vec<Polygon<f64>>, AppError> {
    let geojson_val = if let Some(geom) = body.get("geometry") {
        geom.to_string()
    } else {
        body.to_string()
    };
    let geojson: geojson::GeoJson = geojson_val
        .parse()
        .map_err(|e| AppError::BadRequest(format!("Invalid GeoJSON: {}", e)))?;

    let mut polygons = Vec::new();
    match geojson {
        geojson::GeoJson::Geometry(g) => collect_polygons(&g, &mut polygons)?,
        geojson::GeoJson::Feature(f) => {
            if let Some(g) = f.geometry {
                collect_polygons(&g, &mut polygons)?;
            }
        }
        geojson::GeoJson::FeatureCollection(fc) => {
            for f in fc.features {
                if let Some(g) = f.geometry {
                    collect_polygons(&g, &mut polygons)?;
                }
            }
        }
    }

    if polygons.is_empty() {
        return Err(AppError::BadRequest("No polygons found in input".into()));
    }
    Ok(polygons)
}

fn collect_polygons(
    geom: &geojson::Geometry,
    polygons: &mut Vec<Polygon<f64>>,
) -> Result<(), AppError> {
    let geo_geom: Geometry<f64> = geom
        .clone()
        .try_into()
        .map_err(|e| AppError::Geometry(format!("{}", e)))?;

    match geo_geom {
        Geometry::Polygon(p) => polygons.push(p),
        Geometry::MultiPolygon(mp) => polygons.extend(mp.0),
        _ => {
            return Err(AppError::BadRequest(
                "Expected Polygon or MultiPolygon".into(),
            ))
        }
    }
    Ok(())
}

fn extract_point(val: &Value, label: &str) -> Result<Point<f64>, AppError> {
    let geojson: geojson::Geometry = serde_json::from_value(val.clone())
        .map_err(|e| AppError::BadRequest(format!("Invalid '{}' geometry: {}", label, e)))?;
    let geo_geom: Geometry<f64> = geojson
        .try_into()
        .map_err(|e| AppError::Geometry(format!("{}", e)))?;

    match geo_geom {
        Geometry::Point(p) => Ok(p),
        _ => Err(AppError::BadRequest(format!("'{}' must be a Point", label))),
    }
}

fn extract_points_from_body(body: &Value) -> Result<Vec<Point<f64>>, AppError> {
    let geojson: geojson::GeoJson = body
        .to_string()
        .parse()
        .map_err(|e| AppError::BadRequest(format!("Invalid GeoJSON: {}", e)))?;

    let mut points = Vec::new();
    match geojson {
        geojson::GeoJson::FeatureCollection(fc) => {
            for f in fc.features {
                if let Some(g) = f.geometry {
                    if let Ok(Geometry::Point(p)) = TryInto::<Geometry<f64>>::try_into(g) {
                        points.push(p);
                    }
                }
            }
        }
        _ => return Err(AppError::BadRequest("Expected FeatureCollection".into())),
    }
    Ok(points)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geodesic_area_of_unit_box_at_equator() {
        // A 1° x 1° box at the equator is ~12,308 km² on the WGS 84 ellipsoid.
        let body = json!({
            "geometry": {
                "type": "Polygon",
                "coordinates": [[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [0.0, 0.0]]]
            }
        });
        let polys = extract_polygons_from_body(&body).unwrap();
        assert_eq!(polys.len(), 1);
        let area_km2 = geometry::area::compute_area(&polys).area_km2;
        assert!(
            (12_000.0..12_700.0).contains(&area_km2),
            "expected ~12,308 km², got {area_km2}"
        );
    }

    #[test]
    fn rejects_non_polygon_input() {
        let body = json!({ "type": "Point", "coordinates": [0.0, 0.0] });
        assert!(extract_polygons_from_body(&body).is_err());
    }
}
