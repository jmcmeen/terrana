use crate::error::AppError;
use duckdb::Connection;
use std::path::Path;
use tracing::info;

pub fn ingest_file(conn: &Connection, path: &Path, table: Option<&str>) -> Result<(), AppError> {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    info!(ext = %extension, "ingesting file");

    match extension.as_str() {
        "csv" => {
            conn.execute_batch(&format!(
                "CREATE TABLE data AS SELECT row_number() OVER () AS rowid, * FROM read_csv_auto('{}')",
                path.display()
            ))?;
        }
        "parquet" => {
            conn.execute_batch(&format!(
                "CREATE TABLE data AS SELECT row_number() OVER () AS rowid, * FROM read_parquet('{}')",
                path.display()
            ))?;
        }
        "geojson" | "json" => {
            conn.execute_batch("INSTALL spatial; LOAD spatial;")?;
            conn.execute_batch(&format!(
                "CREATE TABLE data AS SELECT row_number() OVER () AS rowid, * FROM ST_Read('{}')",
                path.display()
            ))?;
        }
        "duckdb" => {
            let tbl = table.ok_or_else(|| {
                AppError::BadRequest("--table is required for .duckdb files".into())
            })?;
            conn.execute_batch(&format!(
                "ATTACH '{}' AS src; CREATE TABLE data AS SELECT row_number() OVER () AS rowid, * FROM src.{}",
                path.display(),
                tbl
            ))?;
        }
        _ => {
            return Err(AppError::BadRequest(format!(
                "Unsupported file type: {}",
                extension
            )));
        }
    }

    let row_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM data", [], |row| row.get(0))?;
    info!(rows = row_count, "file ingested");

    Ok(())
}

const LAT_CANDIDATES: &[&str] = &["latitude", "lat", "y", "ylat", "geo_lat"];
const LON_CANDIDATES: &[&str] = &["longitude", "lon", "lng", "x", "xlon", "xlong", "geo_lon", "geo_lng"];

pub fn detect_lat_lon(
    conn: &Connection,
    lat_override: Option<&str>,
    lon_override: Option<&str>,
) -> Result<(String, String), AppError> {
    let columns = get_column_names(conn)?;
    let col_lower: Vec<String> = columns.iter().map(|c| c.to_lowercase()).collect();

    let lat_col = if let Some(l) = lat_override {
        if !col_lower.contains(&l.to_lowercase()) {
            return Err(AppError::ColumnNotFound(format!(
                "'{}'. Available: {}",
                l,
                columns.join(", ")
            )));
        }
        l.to_string()
    } else {
        detect_column(&columns, &col_lower, LAT_CANDIDATES, "latitude")?
    };

    let lon_col = if let Some(l) = lon_override {
        if !col_lower.contains(&l.to_lowercase()) {
            return Err(AppError::ColumnNotFound(format!(
                "'{}'. Available: {}",
                l,
                columns.join(", ")
            )));
        }
        l.to_string()
    } else {
        detect_column(&columns, &col_lower, LON_CANDIDATES, "longitude")?
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

fn get_column_names(conn: &Connection) -> Result<Vec<String>, AppError> {
    let mut stmt = conn.prepare("SELECT column_name FROM information_schema.columns WHERE table_name = 'data' ORDER BY ordinal_position")?;
    let names: Vec<String> = stmt
        .query_map([], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(names)
}
