//! Response serialization. `format_response` dispatches rows to JSON (default),
//! CSV, or GeoJSON `FeatureCollection` output based on the `format` parameter.

pub mod csv_out;
pub mod geojson_out;
pub mod json_out;

use crate::error::AppError;
use crate::server::AppState;
use axum::response::Response;
use serde_json::Value;

/// Serialize query result `rows` into the requested `format` (`json` | `csv` | `geojson`).
pub fn format_response(
    rows: &[Value],
    format: &str,
    state: &AppState,
) -> Result<Response, AppError> {
    match format {
        "csv" => csv_out::to_csv_response(rows),
        "geojson" => geojson_out::to_geojson_response(rows, state),
        _ => json_out::to_json_response(rows),
    }
}
