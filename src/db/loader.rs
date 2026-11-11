//! File ingestion (CSV / Parquet / GeoJSON / DuckDB), lat/lon auto-detection, and
//! spatial-index construction. All user-supplied identifiers are validated or quoted
//! before being interpolated into SQL.

use crate::db::quote_ident;
use crate::error::AppError;
use duckdb::Connection;
use std::path::Path;
use tracing::{info, warn};

/// Drop the live dataset artifacts. A no-op on first load; called by
/// [`promote_stage`] to clear the previous dataset before swapping in the new one.
///
/// `DETACH DATABASE IF EXISTS source` is a defensive cleanup of the legacy `.duckdb`
/// attach alias; current ingestion attaches under `source_stage` and detaches it in
/// [`stage_file`], so this is normally a no-op.
pub fn drop_dataset(conn: &Connection) -> Result<(), AppError> {
    conn.execute_batch(
        "DROP VIEW IF EXISTS data; \
         DROP INDEX IF EXISTS spatial_idx; \
         DROP TABLE IF EXISTS raw_data; \
         DROP TABLE IF EXISTS spatial_data; \
         DETACH DATABASE IF EXISTS source;",
    )
    .map_err(|e| AppError::Internal(anyhow::anyhow!("Dataset reset error: {}", e)))?;
    Ok(())
}

/// Stage a file into DuckDB as the table `raw_data_stage` (no `data` view yet).
///
/// This is the *risky* half of a (re)load — it reads and parses the external file.
/// Keeping it separate from [`promote_stage`] is what makes `--watch` reloads
/// failure-atomic: if a reload hits a malformed or half-written file, it fails here
/// and the live dataset is left untouched. Adds a `rowid` column via ROW_NUMBER()
/// for spatial-index cross-referencing.
pub fn stage_file(conn: &Connection, path: &Path, table: Option<&str>) -> Result<(), AppError> {
    if !path.exists() {
        return Err(AppError::FileNotFound(path.display().to_string()));
    }
    // Clear any leftovers from a previously-aborted stage before starting fresh.
    discard_stage(conn)?;

    // Best-effort cleanup on failure so a bad stage can't block the next reload.
    if let Err(e) = stage_file_inner(conn, path, table) {
        let _ = discard_stage(conn);
        return Err(e);
    }
    Ok(())
}

fn stage_file_inner(conn: &Connection, path: &Path, table: Option<&str>) -> Result<(), AppError> {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let path_str = path.to_string_lossy();

    info!(ext = %extension, "staging file via DuckDB");

    match extension.as_str() {
        "csv" => {
            conn.execute_batch(&format!(
                "CREATE TABLE raw_data_stage AS SELECT ROW_NUMBER() OVER () AS rowid, * FROM read_csv_auto('{}')",
                escape_sql_string(&path_str)
            ))
            .map_err(|e| AppError::Internal(anyhow::anyhow!("CSV ingestion error: {}", e)))?;
        }
        "parquet" => {
            conn.execute_batch(&format!(
                "CREATE TABLE raw_data_stage AS SELECT ROW_NUMBER() OVER () AS rowid, * FROM read_parquet('{}')",
                escape_sql_string(&path_str)
            ))
            .map_err(|e| AppError::Internal(anyhow::anyhow!("Parquet ingestion error: {}", e)))?;
        }
        "geojson" | "json" => {
            crate::db::ensure_spatial(conn)?;
            conn.execute_batch(&format!(
                "CREATE TABLE raw_data_stage AS SELECT ROW_NUMBER() OVER () AS rowid, * FROM ST_Read('{}')",
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
            // Validate the table identifier before interpolating it into SQL, then
            // quote it as a DuckDB identifier. Prevents injection via --table.
            crate::db::validate_column_name(tbl)?;
            // Attach under a dedicated scratch alias so the live dataset is untouched,
            // copy the table into staging, then detach — the staging copy is self-contained.
            conn.execute_batch(&format!(
                "ATTACH '{}' AS source_stage (READ_ONLY);",
                escape_sql_string(&path_str)
            ))
            .map_err(|e| AppError::Internal(anyhow::anyhow!("DuckDB attach error: {}", e)))?;
            conn.execute_batch(&format!(
                "CREATE TABLE raw_data_stage AS SELECT ROW_NUMBER() OVER () AS rowid, * FROM source_stage.\"{}\"",
                tbl
            ))
            .map_err(|e| AppError::Internal(anyhow::anyhow!("DuckDB table read error: {}", e)))?;
            conn.execute_batch("DETACH DATABASE source_stage;")
                .map_err(|e| AppError::Internal(anyhow::anyhow!("DuckDB detach error: {}", e)))?;
        }
        _ => {
            return Err(AppError::BadRequest(format!(
                "Unsupported file type: {}",
                extension
            )));
        }
    }

    let row_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM raw_data_stage", [], |row| row.get(0))
        .map_err(|e| AppError::Internal(anyhow::anyhow!("COUNT error: {}", e)))?;

    info!(rows = row_count, "file staged into DuckDB");

    Ok(())
}

/// Promote the staged dataset to live: drop the previous dataset, rename
/// `raw_data_stage` → `raw_data`, and (re)create the canonical `data` view that all
/// downstream SQL uses. Call only after [`stage_file`] has succeeded.
pub fn promote_stage(conn: &Connection) -> Result<(), AppError> {
    drop_dataset(conn)?;
    conn.execute_batch(
        "ALTER TABLE raw_data_stage RENAME TO raw_data; \
         CREATE VIEW data AS SELECT * FROM raw_data;",
    )
    .map_err(|e| AppError::Internal(anyhow::anyhow!("Promote staging error: {}", e)))?;
    Ok(())
}

/// Drop a staged-but-not-promoted dataset (and detach its scratch alias). Used to
/// clean up when a reload aborts before promotion, so the next reload starts clean.
pub fn discard_stage(conn: &Connection) -> Result<(), AppError> {
    conn.execute_batch(
        "DROP TABLE IF EXISTS raw_data_stage; \
         DETACH DATABASE IF EXISTS source_stage;",
    )
    .map_err(|e| AppError::Internal(anyhow::anyhow!("Discard staging error: {}", e)))?;
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
        named_column(col_names, l)?
    } else {
        detect_column(col_names, &col_lower, LAT_CANDIDATES, "latitude")?
    };

    let lon_col = if let Some(l) = lon_override {
        named_column(col_names, l)?
    } else {
        detect_column(col_names, &col_lower, LON_CANDIDATES, "longitude")?
    };

    Ok((lat_col, lon_col))
}

/// The header's own spelling of the column an override names (matched as DuckDB
/// matches identifiers), so rows can be read by it; an error listing the columns
/// if there's none.
fn named_column(col_names: &[String], name: &str) -> Result<String, AppError> {
    let names: Vec<&str> = col_names.iter().map(String::as_str).collect();
    match crate::db::resolve_column(name, &names) {
        Ok(i) => Ok(col_names[i].clone()),
        Err(_) => Err(AppError::ColumnNotFound(format!(
            "'{}'. Available: {}",
            name,
            col_names.join(", ")
        ))),
    }
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

/// Add a geometry column and R-tree index to raw_data, then recreate the data view excluding geom.
/// Spatial extension must be loaded after file ingestion to avoid crashes during CSV loading.
pub fn add_spatial_index(
    conn: &duckdb::Connection,
    lat_col: &str,
    lon_col: &str,
) -> Result<(), AppError> {
    info!(lat = %lat_col, lon = %lon_col, "building spatial index");

    crate::db::ensure_spatial(conn)?;

    // One value that isn't a number ("NA", a typo) makes DuckDB read the whole column
    // as text. Such a column is cast to DOUBLE (once, in a subquery); a value that
    // doesn't parse becomes null, and the row is left out, as one with a blank
    // coordinate is.
    let (lat, lon) = (quote_ident(lat_col), quote_ident(lon_col));
    let mut casts = Vec::new();
    for col in [lat_col, lon_col] {
        if column_type(conn, col)? == "VARCHAR" {
            warn_unparsed(conn, col)?;
            let q = quote_ident(col);
            casts.push(format!("TRY_CAST({q} AS DOUBLE) AS {q}"));
        }
    }
    let source = if casts.is_empty() {
        "raw_data".to_string()
    } else {
        format!("(SELECT * REPLACE ({}) FROM raw_data)", casts.join(", "))
    };

    // Create new table with geometry column (avoids ALTER TABLE + UPDATE crash)
    conn.execute_batch(&format!(
        "CREATE TABLE spatial_data AS SELECT *, ST_Point({lon}, {lat}) AS geom FROM {source} WHERE {lat} IS NOT NULL AND {lon} IS NOT NULL",
    ))
    .map_err(|e| AppError::Internal(anyhow::anyhow!("Spatial table creation error: {}", e)))?;

    conn.execute_batch("DROP TABLE raw_data")
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Drop raw_data error: {}", e)))?;
    conn.execute_batch("ALTER TABLE spatial_data RENAME TO raw_data")
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Rename table error: {}", e)))?;

    conn.execute_batch("CREATE INDEX spatial_idx ON raw_data USING RTREE(geom)")
        .map_err(|e| AppError::Internal(anyhow::anyhow!("R-tree index error: {}", e)))?;

    // Recreate data view excluding geom
    conn.execute_batch("DROP VIEW IF EXISTS data")
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Drop view error: {}", e)))?;

    let mut cols = Vec::new();
    let mut stmt = conn
        .prepare("DESCRIBE raw_data")
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
            .map_err(|e| AppError::Internal(anyhow::anyhow!("Column name error: {}", e)))?;
        if name != "geom" {
            cols.push(quote_ident(&name));
        }
    }
    drop(rows);
    drop(stmt);

    conn.execute_batch(&format!(
        "CREATE VIEW data AS SELECT {} FROM raw_data",
        cols.join(", ")
    ))
    .map_err(|e| AppError::Internal(anyhow::anyhow!("Recreate view error: {}", e)))?;

    info!("spatial index created");
    Ok(())
}

/// The DuckDB type of `raw_data`'s column `col` (its exact name: overrides take the
/// header's spelling in [`detect_lat_lon`]).
fn column_type(conn: &duckdb::Connection, col: &str) -> Result<String, AppError> {
    conn.query_row(
        "SELECT data_type FROM information_schema.columns \
         WHERE table_name = 'raw_data' AND column_name = ?",
        [col],
        |row| row.get(0),
    )
    .map_err(|e| AppError::Internal(anyhow::anyhow!("Column type error for '{}': {}", col, e)))
}

/// Log how many of text column `col`'s values aren't numbers, with an example, since
/// those rows are left out.
fn warn_unparsed(conn: &duckdb::Connection, col: &str) -> Result<(), AppError> {
    let q = quote_ident(col);
    let (count, example): (i64, Option<String>) = conn
        .query_row(
            &format!(
                "SELECT count(*), any_value({q}) FROM raw_data \
                 WHERE {q} IS NOT NULL AND TRY_CAST({q} AS DOUBLE) IS NULL"
            ),
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Column check error: {}", e)))?;
    if count > 0 {
        warn!(
            column = %col,
            rows = count,
            example = %example.unwrap_or_default(),
            "values that aren't numbers; those rows are left out"
        );
    }
    Ok(())
}

/// The outcome of [`ingest_file`]: the resolved lat/lon column names and the
/// number of rows loaded.
#[derive(Debug, Clone)]
pub struct IngestInfo {
    pub lat_col: String,
    pub lon_col: String,
    pub row_count: i64,
}

/// Ingest a file end to end: stage it, auto-detect (or validate) the lat & lon
/// columns, promote it to the live `raw_data` table + `data` view, and build the
/// spatial R-tree index.
///
/// This is the library entry point for loading a dataset without the HTTP server.
/// It is failure-atomic in the same way `--watch` reloads are: if column detection
/// fails the staged data is discarded and any previously-loaded dataset is left
/// intact.
pub fn ingest_file(
    conn: &Connection,
    path: &Path,
    table: Option<&str>,
    lat_override: Option<&str>,
    lon_override: Option<&str>,
) -> Result<IngestInfo, AppError> {
    stage_file(conn, path, table)?;

    let staged = crate::db::get_table_info_relation(conn, "raw_data_stage")?;
    let (lat_col, lon_col) = match detect_lat_lon(&staged.col_names, lat_override, lon_override) {
        Ok(cols) => cols,
        Err(e) => {
            let _ = discard_stage(conn);
            return Err(e);
        }
    };

    promote_stage(conn)?;
    add_spatial_index(conn, &lat_col, &lon_col)?;

    Ok(IngestInfo {
        lat_col,
        lon_col,
        row_count: staged.row_count,
    })
}

fn escape_sql_string(s: &str) -> String {
    s.replace('\'', "''")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cols(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn detects_common_names_case_insensitively() {
        let (lat, lon) =
            detect_lat_lon(&cols(&["id", "Latitude", "Longitude"]), None, None).unwrap();
        assert_eq!(lat, "Latitude");
        assert_eq!(lon, "Longitude");

        let (lat, lon) = detect_lat_lon(&cols(&["x", "y", "name"]), None, None).unwrap();
        assert_eq!(lat, "y");
        assert_eq!(lon, "x");
    }

    #[test]
    fn honors_priority_order() {
        // `latitude`/`longitude` outrank `lat`/`lon` when both are present.
        let (lat, lon) =
            detect_lat_lon(&cols(&["lat", "latitude", "lon", "longitude"]), None, None).unwrap();
        assert_eq!(lat, "latitude");
        assert_eq!(lon, "longitude");
    }

    #[test]
    fn an_override_takes_the_headers_spelling() {
        let (lat, lon) = detect_lat_lon(
            &cols(&["Latitude", "Longitude"]),
            Some("latitude"),
            Some("LONGITUDE"),
        )
        .unwrap();
        assert_eq!((lat.as_str(), lon.as_str()), ("Latitude", "Longitude"));
    }

    #[test]
    fn overrides_must_exist() {
        assert!(detect_lat_lon(&cols(&["a", "b"]), Some("a"), Some("b")).is_ok());
        assert!(detect_lat_lon(&cols(&["a", "b"]), Some("missing"), Some("b")).is_err());
    }

    #[test]
    fn detection_fails_without_candidates() {
        assert!(detect_lat_lon(&cols(&["foo", "bar"]), None, None).is_err());
    }

    #[test]
    fn escape_doubles_single_quotes() {
        assert_eq!(escape_sql_string("a'b"), "a''b");
    }

    /// Needs DuckDB's spatial extension: cached after the first download, so offline
    /// from then on.
    #[test]
    fn ingest_file_loads_and_detects_columns() {
        let conn = crate::db::create_connection().unwrap();
        let info = ingest_file(
            &conn,
            Path::new("testdata/observations.csv"),
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(info.lat_col, "latitude");
        assert_eq!(info.lon_col, "longitude");
        assert_eq!(info.row_count, 20);
    }
}
