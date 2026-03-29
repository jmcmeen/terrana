pub mod loader;
pub mod query;

use crate::error::AppError;
use duckdb::Connection;
use std::sync::Mutex;

/// Create an in-memory DuckDB connection with spatial extension loaded.
pub fn create_connection() -> Result<Connection, AppError> {
    let conn = Connection::open_in_memory()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("DuckDB init error: {}", e)))?;
    Ok(conn)
}

/// Schema metadata for a loaded table.
pub struct TableInfo {
    pub col_names: Vec<String>,
    pub col_types: Vec<String>,
    pub row_count: i64,
}

/// Discover schema of the `data` view.
pub fn get_table_info(db: &Mutex<Connection>) -> Result<TableInfo, AppError> {
    let conn = db.lock().unwrap();

    let mut col_names = Vec::new();
    let mut col_types = Vec::new();

    {
        let mut stmt = conn
            .prepare("DESCRIBE data")
            .map_err(|e| AppError::Internal(anyhow::anyhow!("DESCRIBE error: {}", e)))?;
        let mut rows = stmt
            .query([])
            .map_err(|e| AppError::Internal(anyhow::anyhow!("DESCRIBE query error: {}", e)))?;
        while let Some(row) = rows
            .next()
            .map_err(|e| AppError::Internal(anyhow::anyhow!("DESCRIBE row error: {}", e)))?
        {
            let name: String = row.get(0).unwrap();
            let dtype: String = row.get(1).unwrap();
            col_names.push(name);
            col_types.push(dtype);
        }
    }

    let row_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM data", [], |row| row.get(0))
        .map_err(|e| AppError::Internal(anyhow::anyhow!("COUNT error: {}", e)))?;

    Ok(TableInfo {
        col_names,
        col_types,
        row_count,
    })
}

/// Validate a column name from user input (prevent SQL injection).
pub fn validate_column_name(name: &str) -> Result<&str, AppError> {
    if name.is_empty() {
        return Err(AppError::BadRequest("Empty column name not allowed".into()));
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
    {
        return Err(AppError::BadRequest(format!(
            "Invalid column name: '{}'. Only alphanumeric characters and underscores are allowed.",
            name
        )));
    }
    Ok(name)
}
