use crate::index::SpatialPoint;
use crate::store::DataTable;
use rstar::RTree;
use std::time::Instant;
use tracing::info;

pub fn build_rtree(table: &DataTable, lat_col: &str, lon_col: &str) -> RTree<SpatialPoint> {
    let start = Instant::now();
    let capacity = table.row_count.max(0) as usize;

    let mut points = Vec::with_capacity(capacity);
    for (i, row) in table.rows.iter().enumerate() {
        let lat = row.get(lat_col).and_then(|v| v.as_f64());
        let lon = row.get(lon_col).and_then(|v| v.as_f64());
        if let (Some(lat), Some(lon)) = (lat, lon) {
            points.push(SpatialPoint {
                rowid: (i + 1) as i64,
                lat,
                lon,
            });
        }
        if (i + 1) % 10_000_000 == 0 {
            info!(
                collected = points.len(),
                total = capacity,
                elapsed_ms = start.elapsed().as_millis(),
                "collecting points"
            );
        }
    }

    let count = points.len();
    info!(points = count, "building R-tree");
    let tree = RTree::bulk_load(points);
    let elapsed = start.elapsed();

    info!(
        points = count,
        elapsed_ms = elapsed.as_millis(),
        "R-tree index built"
    );

    tree
}
