//! Metadata endpoints: `GET /health`, `GET /schema`, `GET /stats`.

use crate::server::AppState;
use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

/// `GET /health` — liveness probe with process uptime.
pub async fn health(State(state): State<AppState>) -> Json<Value> {
    let uptime = state.start_time.elapsed().as_secs();
    Json(json!({
        "status": "ok",
        "uptime_s": uptime,
    }))
}

/// `GET /schema` — column names/types, lat/lon columns, and row count.
pub async fn schema(State(state): State<AppState>) -> Json<Value> {
    let snap = state.snapshot();
    let s = &snap.schema;
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

/// `GET /stats` — row count, spatial extent, centroid, and index build time.
/// `bbox`/`centroid` are `null` when the dataset has no spatially-valid rows.
pub async fn stats(State(state): State<AppState>) -> Json<Value> {
    let snap = state.snapshot();
    let s = &snap.schema;

    let (bbox, centroid) = match snap.spatial_bbox {
        Some((min_lat, min_lon, max_lat, max_lon)) => (
            json!([min_lat, min_lon, max_lat, max_lon]),
            json!({
                "lat": (min_lat + max_lat) / 2.0,
                "lon": (min_lon + max_lon) / 2.0,
            }),
        ),
        None => (Value::Null, Value::Null),
    };

    Json(json!({
        "row_count": s.row_count,
        "spatial_points": snap.spatial_count,
        "index_build_ms": snap.index_build_ms,
        "bbox": bbox,
        "centroid": centroid,
    }))
}
