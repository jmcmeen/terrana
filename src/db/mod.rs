//! DuckDB connection management, the spatial extension, schema discovery, and the
//! column-name validator shared by all query builders.

pub mod loader;
pub mod query;

use crate::error::AppError;
use duckdb::Connection;
use std::sync::{Mutex, MutexGuard};

/// Load the DuckDB spatial extension.
/// Called after file ingestion, NOT at connection time.
pub fn ensure_spatial(conn: &Connection) -> Result<(), AppError> {
    conn.execute_batch("INSTALL spatial; LOAD spatial;")
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Spatial extension error: {}", e)))?;
    Ok(())
}

/// Create an in-memory DuckDB connection.
pub fn create_connection() -> Result<Connection, AppError> {
    let conn = Connection::open_in_memory()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("DuckDB init error: {}", e)))?;
    Ok(conn)
}

/// Create an on-disk DuckDB connection using a temp file.
/// Reduces RAM usage for large datasets by letting DuckDB spill to disk.
pub fn create_disk_connection() -> Result<(Connection, tempfile::TempDir), AppError> {
    let tmp_dir = tempfile::tempdir()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to create temp dir: {}", e)))?;
    let db_path = tmp_dir.path().join("terrana.duckdb");
    let conn = Connection::open(&db_path)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("DuckDB disk init error: {}", e)))?;
    Ok((conn, tmp_dir))
}

/// Lock the database mutex, mapping poisoned-mutex errors into AppError.
pub fn lock_db(db: &Mutex<Connection>) -> Result<MutexGuard<'_, Connection>, AppError> {
    db.lock()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Database lock poisoned: {}", e)))
}

/// Schema metadata for a loaded table.
pub struct TableInfo {
    pub col_names: Vec<String>,
    pub col_types: Vec<String>,
    pub row_count: i64,
}

/// Discover the schema (column names/types + row count) of an arbitrary relation.
/// `relation` must be a trusted, internal identifier (e.g. `data`, `raw_data_stage`) —
/// it is interpolated directly into SQL and must never come from user input.
pub fn get_table_info_relation(conn: &Connection, relation: &str) -> Result<TableInfo, AppError> {
    let mut col_names = Vec::new();
    let mut col_types = Vec::new();

    {
        let mut stmt = conn
            .prepare(&format!("DESCRIBE {}", relation))
            .map_err(|e| AppError::Internal(anyhow::anyhow!("DESCRIBE error: {}", e)))?;
        let mut rows = stmt
            .query([])
            .map_err(|e| AppError::Internal(anyhow::anyhow!("DESCRIBE query error: {}", e)))?;
        while let Some(row) = rows
            .next()
            .map_err(|e| AppError::Internal(anyhow::anyhow!("DESCRIBE row error: {}", e)))?
        {
            let name: String = row
                .get(0)
                .map_err(|e| AppError::Internal(anyhow::anyhow!("DESCRIBE name: {}", e)))?;
            let dtype: String = row
                .get(1)
                .map_err(|e| AppError::Internal(anyhow::anyhow!("DESCRIBE type: {}", e)))?;
            col_names.push(name);
            col_types.push(dtype);
        }
    }

    let row_count: i64 = conn
        .query_row(&format!("SELECT COUNT(*) FROM {}", relation), [], |row| {
            row.get(0)
        })
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
    if !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return Err(AppError::BadRequest(format!(
            "Invalid column name: '{}'. Only alphanumeric characters and underscores are allowed.",
            name
        )));
    }
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_plain_identifiers() {
        assert!(validate_column_name("species").is_ok());
        assert!(validate_column_name("quality_grade").is_ok());
        assert!(validate_column_name("col1").is_ok());
    }

    #[test]
    fn rejects_empty_and_injection_attempts() {
        assert!(validate_column_name("").is_err());
        assert!(validate_column_name("a; DROP TABLE raw_data").is_err());
        assert!(validate_column_name("a b").is_err());
        assert!(validate_column_name("a\"b").is_err());
        assert!(validate_column_name("a'b").is_err());
        assert!(validate_column_name("a-b").is_err());
    }
}
