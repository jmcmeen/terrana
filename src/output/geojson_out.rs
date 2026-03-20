use crate::error::AppError;
use crate::server::AppState;
use axum::http::header;
use axum::response::{IntoResponse, Response};
use serde_json::{json, Value};

pub fn to_geojson_response(rows: &[Value], state: &AppState) -> Result<Response, AppError> {
    let lat_col = &state.schema.lat_col;
    let lon_col = &state.schema.lon_col;

    let features: Vec<Value> = rows
        .iter()
        .filter_map(|row| {
            let obj = row.as_object()?;
            let lat = obj.get(lat_col).and_then(|v| v.as_f64())?;
            let lon = obj.get(lon_col).and_then(|v| v.as_f64())?;

            let mut properties = serde_json::Map::new();
            for (k, v) in obj {
                if k != lat_col && k != lon_col {
                    properties.insert(k.clone(), v.clone());
                }
            }

            Some(json!({
                "type": "Feature",
                "geometry": {
                    "type": "Point",
                    "coordinates": [lon, lat]
                },
                "properties": properties,
            }))
        })
        .collect();

    let fc = json!({
        "type": "FeatureCollection",
        "features": features,
    });

    let body = serde_json::to_string(&fc)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("GeoJSON serialization error: {}", e)))?;

    Ok(([(header::CONTENT_TYPE, "application/geo+json")], body).into_response())
}
