use crate::db;
use crate::error::AppError;
use crate::output;
use crate::server::AppState;
use axum::extract::State;
use axum::response::Response;
use axum::Json;
use geojson::GeoJson;

pub async fn within(
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> Result<Response, AppError> {
    // Extract the GeoJSON geometry and use ST_Contains for R-tree accelerated PIP
    let geojson_str = extract_geometry_geojson(&body)?;
    let spatial = db::query::within_filter_geojson(&geojson_str);

    let rows = db::query::query(
        &state.db,
        Some(&spatial),
        &[],
        None,
        None,
        None,
        100_000,
        None,
        None,
    )?;

    output::format_response(&rows, "json", &state)
}

/// Extract the geometry portion from the input body as a GeoJSON string.
/// Supports Polygon, MultiPolygon, Feature, or FeatureCollection.
fn extract_geometry_geojson(body: &serde_json::Value) -> Result<String, AppError> {
    let geojson: GeoJson = body
        .to_string()
        .parse::<GeoJson>()
        .map_err(|e| AppError::BadRequest(format!("Invalid GeoJSON: {}", e)))?;

    // We need to extract just the geometry as a GeoJSON string for ST_GeomFromGeoJSON.
    // For multi-polygon or feature collections with multiple polygons, we union them.
    match geojson {
        GeoJson::Geometry(ref geom) => {
            validate_polygon_geometry(geom)?;
            Ok(geom.to_string())
        }
        GeoJson::Feature(ref feat) => {
            let geom = feat
                .geometry
                .as_ref()
                .ok_or_else(|| AppError::BadRequest("Feature has no geometry".into()))?;
            validate_polygon_geometry(geom)?;
            Ok(geom.to_string())
        }
        GeoJson::FeatureCollection(ref fc) => {
            // Collect all polygon geometries and create a single MultiPolygon
            let mut all_coords: Vec<Vec<Vec<Vec<f64>>>> = Vec::new();
            for feat in &fc.features {
                if let Some(geom) = &feat.geometry {
                    match &geom.value {
                        geojson::Value::Polygon(coords) => {
                            all_coords.push(coords.clone());
                        }
                        geojson::Value::MultiPolygon(multi) => {
                            all_coords.extend(multi.clone());
                        }
                        _ => {
                            return Err(AppError::BadRequest(
                                "Expected Polygon or MultiPolygon geometry".into(),
                            ));
                        }
                    }
                }
            }
            if all_coords.is_empty() {
                return Err(AppError::BadRequest("No polygons found in input".into()));
            }
            let multi = geojson::Geometry::new(geojson::Value::MultiPolygon(all_coords));
            Ok(multi.to_string())
        }
    }
}

fn validate_polygon_geometry(geom: &geojson::Geometry) -> Result<(), AppError> {
    match &geom.value {
        geojson::Value::Polygon(_) | geojson::Value::MultiPolygon(_) => Ok(()),
        _ => Err(AppError::BadRequest(
            "Expected Polygon or MultiPolygon geometry".into(),
        )),
    }
}
