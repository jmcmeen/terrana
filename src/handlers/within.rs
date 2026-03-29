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
    // Extract the GeoJSON geometry and use ST_Contains (R-tree accelerated)
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

    match geojson {
        GeoJson::Geometry(geom) => {
            validate_polygon_geometry(&geom)?;
            Ok(geom.to_string())
        }
        GeoJson::Feature(feat) => {
            let geom = feat
                .geometry
                .ok_or_else(|| AppError::BadRequest("Feature has no geometry".into()))?;
            validate_polygon_geometry(&geom)?;
            Ok(geom.to_string())
        }
        GeoJson::FeatureCollection(fc) => {
            // For a collection, pass through the first polygon geometry found,
            // or combine into a single geometry for ST_Contains.
            // Simplest: just use the raw body as-is isn't valid for ST_GeomFromGeoJSON,
            // so extract all polygon geometries and merge into a MultiPolygon.
            let mut all_polys: Vec<serde_json::Value> = Vec::new();
            for feat in &fc.features {
                if let Some(geom) = &feat.geometry {
                    validate_polygon_geometry(geom)?;
                    // Parse the geometry coordinates
                    let geom_json: serde_json::Value = serde_json::to_value(geom)
                        .map_err(|e| AppError::Internal(anyhow::anyhow!("JSON error: {}", e)))?;
                    match geom_json.get("type").and_then(|t| t.as_str()) {
                        Some("Polygon") => {
                            if let Some(coords) = geom_json.get("coordinates") {
                                all_polys.push(coords.clone());
                            }
                        }
                        Some("MultiPolygon") => {
                            if let Some(coords) = geom_json.get("coordinates").and_then(|c| c.as_array()) {
                                for poly_coords in coords {
                                    all_polys.push(poly_coords.clone());
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            if all_polys.is_empty() {
                return Err(AppError::BadRequest("No polygons found in input".into()));
            }
            let multi = serde_json::json!({
                "type": "MultiPolygon",
                "coordinates": all_polys,
            });
            Ok(multi.to_string())
        }
    }
}

fn validate_polygon_geometry(geom: &geojson::Geometry) -> Result<(), AppError> {
    let geom_json: serde_json::Value = serde_json::to_value(geom)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("JSON error: {}", e)))?;
    match geom_json.get("type").and_then(|t| t.as_str()) {
        Some("Polygon") | Some("MultiPolygon") => Ok(()),
        _ => Err(AppError::BadRequest(
            "Expected Polygon or MultiPolygon geometry".into(),
        )),
    }
}
