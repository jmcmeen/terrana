use crate::server::AppState;
use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

pub async fn health(State(state): State<AppState>) -> Json<Value> {
    let uptime = state.start_time.elapsed().as_secs();
    Json(json!({
        "status": "ok",
        "uptime_s": uptime,
    }))
}

pub async fn schema(State(state): State<AppState>) -> Json<Value> {
    let s = &state.schema;
    Json(json!({
        "source": s.source,
        "row_count": s.row_count,
        "lat_column": s.lat_col,
        "lon_column": s.lon_col,
        "columns": s.columns.iter().map(|c| json!({
            "name": c.name,
            "type": c.dtype,
        })).collect::<Vec<_>>(),
    }))
}

pub async fn stats(State(state): State<AppState>) -> Json<Value> {
    let s = &state.schema;
    let (min_lat, min_lon, max_lat, max_lon) = state.spatial_bbox.unwrap_or((0.0, 0.0, 0.0, 0.0));

    let centroid_lat = (min_lat + max_lat) / 2.0;
    let centroid_lon = (min_lon + max_lon) / 2.0;

    Json(json!({
        "row_count": s.row_count,
        "spatial_points": state.spatial_count,
        "index_build_ms": state.index_build_ms,
        "bbox": [min_lat, min_lon, max_lat, max_lon],
        "centroid": {
            "lat": centroid_lat,
            "lon": centroid_lon,
        },
    }))
}
