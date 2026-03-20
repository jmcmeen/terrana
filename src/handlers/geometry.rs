use crate::error::AppError;
use crate::server::AppState;
use axum::extract::State;
use axum::Json;
use geo::{
    Bearing, BoundingRect, Centroid, ConvexHull, Destination, Distance, Geodesic, GeodesicArea,
    Simplify, SimplifyVw,
};
use geo_types::{LineString, Point, Polygon};
use serde_json::{json, Value};
use std::convert::TryInto;

pub async fn area(
    State(_state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let polygons = extract_polygons_from_body(&body)?;

    let mut total_area_m2 = 0.0_f64;
    let mut total_perimeter_m = 0.0_f64;

    for poly in &polygons {
        total_area_m2 += poly.geodesic_area_unsigned();
        total_perimeter_m += poly.geodesic_perimeter();
    }

    Ok(Json(json!({
        "area_m2": total_area_m2,
        "area_km2": total_area_m2 / 1_000_000.0,
        "area_ha": total_area_m2 / 10_000.0,
        "area_acres": total_area_m2 / 4_046.8564224,
        "perimeter_m": total_perimeter_m,
    })))
}

pub async fn convex_hull(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let points = if let Some(query) = body.get("query") {
        // Build from spatial query
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
        let (min_lat, min_lon, max_lat, max_lon) = (coords[0], coords[1], coords[2], coords[3]);

        let envelope = rstar::AABB::from_corners([min_lon, min_lat], [max_lon, max_lat]);
        let pts: Vec<Point<f64>> = state
            .index
            .locate_in_envelope(&envelope)
            .map(|p| Point::new(p.lon, p.lat))
            .collect();
        pts
    } else {
        // Build from GeoJSON features
        extract_points_from_body(&body)?
    };

    if points.len() < 3 {
        return Err(AppError::BadRequest(
            "Need at least 3 points for convex hull".into(),
        ));
    }

    let multi_point = geo_types::MultiPoint::from(points.clone());
    let hull = multi_point.convex_hull();

    let area_m2 = hull.geodesic_area_unsigned();
    let perimeter_m = hull.geodesic_perimeter();

    let hull_geojson: geojson::Geometry = (&hull).into();

    Ok(Json(json!({
        "type": "Feature",
        "geometry": serde_json::to_value(&hull_geojson).unwrap_or(json!(null)),
        "properties": {
            "area_m2": area_m2,
            "area_km2": area_m2 / 1_000_000.0,
            "area_ha": area_m2 / 10_000.0,
            "perimeter_m": perimeter_m,
            "point_count": points.len(),
        }
    })))
}

pub async fn centroid(
    State(_state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
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

    let geo_geom: geo_types::Geometry<f64> = geom
        .try_into()
        .map_err(|e| AppError::Geometry(format!("{}", e)))?;

    let c = geo_geom
        .centroid()
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

    let geojson: geojson::Geometry = serde_json::from_value(geom_val.clone())
        .map_err(|e| AppError::BadRequest(format!("Invalid geometry: {}", e)))?;
    let geo_geom: geo_types::Geometry<f64> = geojson
        .try_into()
        .map_err(|e| AppError::Geometry(format!("{}", e)))?;

    let center = geo_geom
        .centroid()
        .ok_or_else(|| AppError::Geometry("Cannot compute centroid for buffer".into()))?;

    // Build buffer ring by shooting rays at each bearing
    let mut coords = Vec::with_capacity(segments + 1);
    for i in 0..segments {
        let bearing = (i as f64) * 360.0 / (segments as f64);
        let dest = Geodesic::destination(center, bearing, distance_m);
        coords.push(geo_types::Coord {
            x: dest.x(),
            y: dest.y(),
        });
    }
    coords.push(coords[0]); // close the ring

    let ring = LineString::from(coords);
    let poly = Polygon::new(ring, vec![]);

    let area_m2 = poly.geodesic_area_unsigned();
    let poly_geojson: geojson::Geometry = (&poly).into();

    Ok(Json(json!({
        "type": "Feature",
        "geometry": serde_json::to_value(&poly_geojson).unwrap_or(json!(null)),
        "properties": {
            "area_m2": area_m2,
            "area_km2": area_m2 / 1_000_000.0,
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

    // Get points from query or all
    let points_with_data = if let Some(query) = body.get("query") {
        let bbox = query
            .get("bbox")
            .and_then(|v| v.as_array())
            .ok_or_else(|| AppError::BadRequest("Expected query.bbox".into()))?;
        let mut coords = [0.0_f64; 4];
        for (i, coord) in coords.iter_mut().enumerate() {
            *coord = bbox[i]
                .as_f64()
                .ok_or_else(|| AppError::BadRequest("bbox values must be numeric".into()))?;
        }
        let (min_lat, min_lon, max_lat, max_lon) = (coords[0], coords[1], coords[2], coords[3]);

        let envelope = rstar::AABB::from_corners([min_lon, min_lat], [max_lon, max_lat]);
        let rowids: Vec<i64> = state
            .index
            .locate_in_envelope(&envelope)
            .map(|p| p.rowid)
            .collect();

        let db = state
            .db
            .lock()
            .map_err(|e| AppError::Internal(anyhow::anyhow!("{}", e)))?;
        crate::db::query::fetch_rows_by_ids(&db, &rowids, None)?
    } else {
        let db = state
            .db
            .lock()
            .map_err(|e| AppError::Internal(anyhow::anyhow!("{}", e)))?;
        crate::db::query::fetch_all_rows(&db, &[], None, None, None, 100_000)?
    };

    // Group by attribute
    let lat_col = &state.schema.lat_col;
    let lon_col = &state.schema.lon_col;
    let mut groups: std::collections::HashMap<String, Vec<Point<f64>>> =
        std::collections::HashMap::new();

    for row in &points_with_data {
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

    // Build hull per group
    let mut features = Vec::new();
    for (key, pts) in &groups {
        if pts.len() < 3 {
            continue;
        }
        let multi_point = geo_types::MultiPoint::from(pts.clone());
        let hull = multi_point.convex_hull();
        let hull_geojson: geojson::Geometry = (&hull).into();

        let mut props = serde_json::Map::new();
        props.insert(by.to_string(), json!(key));
        if include_count {
            props.insert("count".to_string(), json!(pts.len()));
        }
        if include_area {
            let area_m2 = hull.geodesic_area_unsigned();
            props.insert("area_m2".to_string(), json!(area_m2));
            props.insert("area_km2".to_string(), json!(area_m2 / 1_000_000.0));
        }

        features.push(json!({
            "type": "Feature",
            "geometry": serde_json::to_value(&hull_geojson).unwrap_or(json!(null)),
            "properties": props,
        }));
    }

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

    let geojson: geojson::Geometry = serde_json::from_value(geom_val.clone())
        .map_err(|e| AppError::BadRequest(format!("Invalid geometry: {}", e)))?;
    let geo_geom: geo_types::Geometry<f64> = geojson
        .try_into()
        .map_err(|e| AppError::Geometry(format!("{}", e)))?;

    let simplified = match geo_geom {
        geo_types::Geometry::Polygon(p) => {
            if preserve_topology {
                geo_types::Geometry::Polygon(p.simplify_vw(&tolerance))
            } else {
                geo_types::Geometry::Polygon(p.simplify(&tolerance))
            }
        }
        geo_types::Geometry::MultiPolygon(mp) => {
            if preserve_topology {
                geo_types::Geometry::MultiPolygon(mp.simplify_vw(&tolerance))
            } else {
                geo_types::Geometry::MultiPolygon(mp.simplify(&tolerance))
            }
        }
        geo_types::Geometry::LineString(ls) => {
            if preserve_topology {
                geo_types::Geometry::LineString(ls.simplify_vw(&tolerance))
            } else {
                geo_types::Geometry::LineString(ls.simplify(&tolerance))
            }
        }
        other => other,
    };

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

    let distance_m = Geodesic::distance(from_pt, to_pt);
    let bearing = Geodesic::bearing(from_pt, to_pt);

    Ok(Json(json!({
        "distance_m": distance_m,
        "distance_km": distance_m / 1000.0,
        "distance_mi": distance_m / 1609.344,
        "bearing_deg": bearing,
    })))
}

pub async fn bounds(
    State(_state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
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

    let geo_geom: geo_types::Geometry<f64> = geom
        .try_into()
        .map_err(|e| AppError::Geometry(format!("{}", e)))?;

    let rect = geo_geom
        .bounding_rect()
        .ok_or_else(|| AppError::Geometry("Could not compute bounding rect".into()))?;

    let min_lat = rect.min().y;
    let min_lon = rect.min().x;
    let max_lat = rect.max().y;
    let max_lon = rect.max().x;

    // Compute width/height in km using geodesic distance
    let width_m = Geodesic::distance(Point::new(min_lon, min_lat), Point::new(max_lon, min_lat));
    let height_m = Geodesic::distance(Point::new(min_lon, min_lat), Point::new(min_lon, max_lat));

    let envelope_poly = Polygon::new(
        LineString::from(vec![
            (min_lon, min_lat),
            (max_lon, min_lat),
            (max_lon, max_lat),
            (min_lon, max_lat),
            (min_lon, min_lat),
        ]),
        vec![],
    );
    let envelope_geojson: geojson::Geometry = (&envelope_poly).into();
    let area_m2 = envelope_poly.geodesic_area_unsigned();

    Ok(Json(json!({
        "bbox": [min_lat, min_lon, max_lat, max_lon],
        "envelope": serde_json::to_value(&envelope_geojson).unwrap_or(json!(null)),
        "width_km": width_m / 1000.0,
        "height_km": height_m / 1000.0,
        "area_km2": area_m2 / 1_000_000.0,
    })))
}

// --- Helpers ---

fn extract_polygons_from_body(body: &Value) -> Result<Vec<Polygon<f64>>, AppError> {
    // Support both {"geometry": <GeoJSON>} wrapper and direct GeoJSON
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
    let geo_geom: geo_types::Geometry<f64> = geom
        .clone()
        .try_into()
        .map_err(|e| AppError::Geometry(format!("{}", e)))?;

    match geo_geom {
        geo_types::Geometry::Polygon(p) => polygons.push(p),
        geo_types::Geometry::MultiPolygon(mp) => polygons.extend(mp.0),
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
    let geo_geom: geo_types::Geometry<f64> = geojson
        .try_into()
        .map_err(|e| AppError::Geometry(format!("{}", e)))?;

    match geo_geom {
        geo_types::Geometry::Point(p) => Ok(p),
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
                    if let Ok(geo_types::Geometry::Point(p)) =
                        TryInto::<geo_types::Geometry<f64>>::try_into(g)
                    {
                        points.push(p);
                    }
                }
            }
        }
        _ => return Err(AppError::BadRequest("Expected FeatureCollection".into())),
    }
    Ok(points)
}
