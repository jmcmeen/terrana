use crate::error::AppError;
use crate::output;
use crate::server::AppState;
use axum::extract::State;
use axum::Json;
use axum::response::Response;
use geo::Contains;
use geo_types::Polygon;
use geojson::GeoJson;

pub async fn within(
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> Result<Response, AppError> {
    let polygons = extract_polygons(&body)?;

    // Test each point in the index
    let matching_rowids: Vec<i64> = state
        .index
        .iter()
        .filter(|pt| {
            let point = geo_types::Point::new(pt.lon, pt.lat);
            polygons.iter().any(|poly| poly.contains(&point))
        })
        .map(|pt| pt.rowid)
        .collect();

    let db = state.db.lock().map_err(|e| AppError::Internal(anyhow::anyhow!("{}", e)))?;
    let rows = crate::db::query::fetch_rows_by_ids(&db, &matching_rowids, None)?;
    drop(db);

    output::format_response(&rows, "json", &state)
}

fn extract_polygons(body: &serde_json::Value) -> Result<Vec<Polygon<f64>>, AppError> {
    let geojson: GeoJson = body
        .to_string()
        .parse::<GeoJson>()
        .map_err(|e| AppError::BadRequest(format!("Invalid GeoJSON: {}", e)))?;

    let mut polygons = Vec::new();

    match geojson {
        GeoJson::Geometry(geom) => {
            collect_polygons_from_geometry(&geom, &mut polygons)?;
        }
        GeoJson::Feature(feat) => {
            if let Some(geom) = feat.geometry {
                collect_polygons_from_geometry(&geom, &mut polygons)?;
            }
        }
        GeoJson::FeatureCollection(fc) => {
            for feat in fc.features {
                if let Some(geom) = feat.geometry {
                    collect_polygons_from_geometry(&geom, &mut polygons)?;
                }
            }
        }
    }

    if polygons.is_empty() {
        return Err(AppError::BadRequest("No polygons found in input".into()));
    }

    Ok(polygons)
}

fn collect_polygons_from_geometry(
    geom: &geojson::Geometry,
    polygons: &mut Vec<Polygon<f64>>,
) -> Result<(), AppError> {
    use std::convert::TryInto;

    let geo_geom: geo_types::Geometry<f64> = geom
        .clone()
        .try_into()
        .map_err(|e| AppError::Geometry(format!("Failed to convert geometry: {}", e)))?;

    match geo_geom {
        geo_types::Geometry::Polygon(p) => polygons.push(p),
        geo_types::Geometry::MultiPolygon(mp) => {
            for p in mp.0 {
                polygons.push(p);
            }
        }
        _ => {
            return Err(AppError::BadRequest(
                "Expected Polygon or MultiPolygon geometry".into(),
            ));
        }
    }

    Ok(())
}
