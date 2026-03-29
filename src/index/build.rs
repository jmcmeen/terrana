use crate::db;
use crate::error::AppError;
use crate::index::SpatialPoint;
use duckdb::Connection;
use rstar::RTree;
use std::sync::Mutex;
use std::time::Instant;
use tracing::info;

pub fn build_rtree(
    db_conn: &Mutex<Connection>,
    lat_col: &str,
    lon_col: &str,
) -> Result<RTree<SpatialPoint>, AppError> {
    let start = Instant::now();

    let raw_points = db::query::scan_lat_lon(db_conn, lat_col, lon_col)?;

    let points: Vec<SpatialPoint> = raw_points
        .into_iter()
        .map(|(rowid, lat, lon)| SpatialPoint { rowid, lat, lon })
        .collect();

    let count = points.len();
    info!(points = count, "building R-tree");
    let tree = RTree::bulk_load(points);

    info!(
        points = count,
        elapsed_ms = start.elapsed().as_millis(),
        "R-tree index built"
    );

    Ok(tree)
}
