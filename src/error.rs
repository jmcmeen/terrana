use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(thiserror::Error, Debug)]
pub enum AppError {
    #[error("Invalid query parameter: {0}")]
    BadRequest(String),
    #[error("Column not found: {0}")]
    ColumnNotFound(String),
    #[error("File not found: {0}")]
    #[allow(dead_code)]
    FileNotFound(String),
    #[error("Geometry error: {0}")]
    Geometry(String),
    #[error("Internal error: {0}")]
    Internal(#[from] anyhow::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::BadRequest(_) | AppError::ColumnNotFound(_) => {
                (StatusCode::BAD_REQUEST, self.to_string())
            }
            AppError::FileNotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            AppError::Geometry(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            AppError::Internal(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        };

        let body = json!({ "error": message });
        (status, axum::Json(body)).into_response()
    }
}
