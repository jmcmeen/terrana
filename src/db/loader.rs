use crate::error::AppError;
use duckdb::Connection;
use std::path::Path;
use tracing::info;

/// Ingest a file into DuckDB as a view named `data`.
/// Adds a `rowid` column via ROW_NUMBER() for spatial index cross-referencing.
pub fn load_file(conn: &Connection, path: &Path, table: Option<&str>) -> Result<(), AppError> {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let path_str = path.to_string_lossy();

    info!(ext = %extension, "loading file via DuckDB");

    match extension.as_str() {
        "csv" => {
            conn.execute_batch(&format!(
                "CREATE TABLE raw_data AS SELECT ROW_NUMBER() OVER () AS rowid, * FROM read_csv_auto('{}')",
                escape_sql_string(&path_str)
            ))
            .map_err(|e| AppError::Internal(anyhow::anyhow!("CSV ingestion error: {}", e)))?;
        }
        "parquet" => {
            conn.execute_batch(&format!(
                "CREATE TABLE raw_data AS SELECT ROW_NUMBER() OVER () AS rowid, * FROM read_parquet('{}')",
                escape_sql_string(&path_str)
            ))
            .map_err(|e| AppError::Internal(anyhow::anyhow!("Parquet ingestion error: {}", e)))?;
        }
        "geojson" | "json" => {
            conn.execute_batch("INSTALL spatial; LOAD spatial;")
                .map_err(|e| {
                    AppError::Internal(anyhow::anyhow!("Spatial extension error: {}", e))
                })?;
            conn.execute_batch(&format!(
                "CREATE TABLE raw_data AS SELECT ROW_NUMBER() OVER () AS rowid, * FROM ST_Read('{}')",
                escape_sql_string(&path_str)
            ))
            .map_err(|e| AppError::Internal(anyhow::anyhow!("GeoJSON ingestion error: {}", e)))?;
        }
        "duckdb" => {
            let tbl = table.ok_or_else(|| {
                AppError::BadRequest(
                    "For .duckdb files, specify --table <TABLE> to select the table".into(),
                )
            })?;
            // Attach the file and create a view
            conn.execute_batch(&format!(
                "ATTACH '{}' AS source (READ_ONLY);",
                escape_sql_string(&path_str)
            ))
            .map_err(|e| AppError::Internal(anyhow::anyhow!("DuckDB attach error: {}", e)))?;
            conn.execute_batch(&format!(
                "CREATE TABLE raw_data AS SELECT ROW_NUMBER() OVER () AS rowid, * FROM source.{}",
                tbl
            ))
            .map_err(|e| AppError::Internal(anyhow::anyhow!("DuckDB table read error: {}", e)))?;
        }
        _ => {
            return Err(AppError::BadRequest(format!(
                "Unsupported file type: {}",
                extension
            )));
        }
    }

    // Create the canonical `data` view that all downstream SQL uses
    conn.execute_batch("CREATE VIEW data AS SELECT * FROM raw_data")
        .map_err(|e| AppError::Internal(anyhow::anyhow!("View creation error: {}", e)))?;

    let row_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM data", [], |row| row.get(0))
        .map_err(|e| AppError::Internal(anyhow::anyhow!("COUNT error: {}", e)))?;

    info!(rows = row_count, "file loaded into DuckDB");

    Ok(())
}

const LAT_CANDIDATES: &[&str] = &["latitude", "lat", "y", "ylat", "geo_lat"];
const LON_CANDIDATES: &[&str] = &[
    "longitude",
    "lon",
    "lng",
    "x",
    "xlon",
    "xlong",
    "geo_lon",
    "geo_lng",
];

/// Detect lat/lon column names from the schema.
pub fn detect_lat_lon(
    col_names: &[String],
    lat_override: Option<&str>,
    lon_override: Option<&str>,
) -> Result<(String, String), AppError> {
    let col_lower: Vec<String> = col_names.iter().map(|c| c.to_lowercase()).collect();

    let lat_col = if let Some(l) = lat_override {
        if !col_lower.contains(&l.to_lowercase()) {
            return Err(AppError::ColumnNotFound(format!(
                "'{}'. Available: {}",
                l,
                col_names.join(", ")
            )));
        }
        l.to_string()
    } else {
        detect_column(col_names, &col_lower, LAT_CANDIDATES, "latitude")?
    };

    let lon_col = if let Some(l) = lon_override {
        if !col_lower.contains(&l.to_lowercase()) {
            return Err(AppError::ColumnNotFound(format!(
                "'{}'. Available: {}",
                l,
                col_names.join(", ")
            )));
        }
        l.to_string()
    } else {
        detect_column(col_names, &col_lower, LON_CANDIDATES, "longitude")?
    };

    Ok((lat_col, lon_col))
}

fn detect_column(
    columns: &[String],
    col_lower: &[String],
    candidates: &[&str],
    label: &str,
) -> Result<String, AppError> {
    for candidate in candidates {
        if let Some(idx) = col_lower.iter().position(|c| c == candidate) {
            return Ok(columns[idx].clone());
        }
    }
    Err(AppError::BadRequest(format!(
        "Could not auto-detect {} column. Available columns: {}. Use --lat/--lon to specify.",
        label,
        columns.join(", ")
    )))
}

fn escape_sql_string(s: &str) -> String {
    s.replace('\'', "''")
}
