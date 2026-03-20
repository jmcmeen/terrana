use crate::error::AppError;
use axum::http::header;
use axum::response::{IntoResponse, Response};
use serde_json::Value;

pub fn to_csv_response(rows: &[Value]) -> Result<Response, AppError> {
    if rows.is_empty() {
        return Ok(([(header::CONTENT_TYPE, "text/csv")], "").into_response());
    }

    let mut wtr = csv::Writer::from_writer(vec![]);

    // Extract headers from first row
    let headers: Vec<String> = if let Some(obj) = rows[0].as_object() {
        obj.keys().cloned().collect()
    } else {
        return Err(AppError::Internal(anyhow::anyhow!("Expected object rows")));
    };

    wtr.write_record(&headers)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("CSV error: {}", e)))?;

    for row in rows {
        if let Some(obj) = row.as_object() {
            let record: Vec<String> = headers
                .iter()
                .map(|h| {
                    obj.get(h)
                        .map(|v| match v {
                            Value::String(s) => s.clone(),
                            Value::Null => String::new(),
                            other => other.to_string(),
                        })
                        .unwrap_or_default()
                })
                .collect();
            wtr.write_record(&record)
                .map_err(|e| AppError::Internal(anyhow::anyhow!("CSV error: {}", e)))?;
        }
    }

    let data = wtr
        .into_inner()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("CSV flush error: {}", e)))?;
    let body = String::from_utf8(data)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("CSV UTF-8 error: {}", e)))?;

    Ok(([(header::CONTENT_TYPE, "text/csv")], body).into_response())
}
