use crate::error::AppError;
use axum::http::header;
use axum::response::{IntoResponse, Response};
use serde_json::Value;

pub fn to_json_response(rows: &[Value]) -> Result<Response, AppError> {
    let body = serde_json::to_string(rows)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("JSON serialization error: {}", e)))?;
    Ok(([(header::CONTENT_TYPE, "application/json")], body).into_response())
}
