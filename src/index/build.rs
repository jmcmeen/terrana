use crate::error::AppError;
use crate::index::SpatialPoint;
use duckdb::Connection;
use rstar::RTree;
use std::time::Instant;
use tracing::info;

pub fn build_rtree(
    conn: &Connection,
    lat_col: &str,
    lon_col: &str,
) -> Result<RTree<SpatialPoint>, AppError> {
    let start = Instant::now();

    let quoted_lat = crate::db::query::quote_identifier(lat_col)?;
    let quoted_lon = crate::db::query::quote_identifier(lon_col)?;
    let sql = format!(
        "SELECT rowid, {lat}, {lon} FROM data WHERE {lat} IS NOT NULL AND {lon} IS NOT NULL",
        lat = quoted_lat,
        lon = quoted_lon,
    );

    let mut stmt = conn.prepare(&sql)?;
    let points: Vec<SpatialPoint> = stmt
        .query_map([], |row| {
            Ok(SpatialPoint {
                rowid: row.get(0)?,
                lat: row.get(1)?,
                lon: row.get(2)?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();

    let count = points.len();
    let tree = RTree::bulk_load(points);
    let elapsed = start.elapsed();

    info!(
        points = count,
        elapsed_ms = elapsed.as_millis(),
        "R-tree index built"
    );

    Ok(tree)
}
