//! Axum router assembly and the shared [`AppState`] passed to every handler.

pub mod middleware;

use crate::config::Config;
use crate::error::AppError;
use crate::handlers;
use axum::Router;
use duckdb::Connection;
use serde::Serialize;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

/// State shared across all handlers. Cloning is cheap — every expensive field is
/// behind an `Arc`. The mutable, refreshable parts (schema + spatial stats) live in
/// [`Snapshot`] behind an `RwLock` so `--watch` can swap them atomically on reload.
#[derive(Clone)]
pub struct AppState {
    /// Resolved CLI configuration. Retained for introspection / future handlers.
    #[allow(dead_code)]
    pub config: Arc<Config>,
    pub db: Arc<Mutex<Connection>>,
    pub snapshot: Arc<RwLock<Arc<Snapshot>>>,
    pub start_time: Instant,
}

impl AppState {
    /// Take a cheap, lock-free-after-clone view of the current dataset snapshot.
    /// Recovers from a poisoned lock so a panicking reload can never wedge reads.
    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.snapshot
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

/// A geographic bounding box: `(min_lat, min_lon, max_lat, max_lon)`.
pub type BBox = (f64, f64, f64, f64);

/// An immutable view of the loaded dataset. Swapped wholesale on `--watch` reload.
pub struct Snapshot {
    pub schema: TableSchema,
    pub index_build_ms: u128,
    pub spatial_bbox: Option<BBox>,
    pub spatial_count: i64,
}

#[derive(Debug, Serialize)]
pub struct TableSchema {
    pub source: String,
    pub row_count: i64,
    pub lat_col: String,
    pub lon_col: String,
    pub columns: Vec<ColumnMeta>,
}

#[derive(Debug, Serialize)]
pub struct ColumnMeta {
    pub name: String,
    pub dtype: String,
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", axum::routing::get(handlers::meta::health))
        .route("/schema", axum::routing::get(handlers::meta::schema))
        .route("/stats", axum::routing::get(handlers::meta::stats))
        .route("/query", axum::routing::get(handlers::query::query))
        .route(
            "/query/within",
            axum::routing::post(handlers::within::within),
        )
        .route(
            "/geometry/convex-hull",
            axum::routing::post(handlers::geometry::convex_hull),
        )
        .route(
            "/geometry/area",
            axum::routing::post(handlers::geometry::area),
        )
        .route(
            "/geometry/centroid",
            axum::routing::post(handlers::geometry::centroid),
        )
        .route(
            "/geometry/buffer",
            axum::routing::post(handlers::geometry::buffer),
        )
        .route(
            "/geometry/dissolve",
            axum::routing::post(handlers::geometry::dissolve),
        )
        .route(
            "/geometry/simplify",
            axum::routing::post(handlers::geometry::simplify),
        )
        .route(
            "/geometry/distance",
            axum::routing::post(handlers::geometry::distance),
        )
        .route(
            "/geometry/bounds",
            axum::routing::post(handlers::geometry::bounds),
        )
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Assemble a [`Snapshot`] from an already-ingested connection.
///
/// `conn` must already have the live `raw_data` table + `data` view in place
/// (e.g. via [`crate::ingest_file`]). This reads the `data` view's columns,
/// computes the spatial extent, and packages them with the caller-supplied
/// `row_count` (the full pre-index count) and `index_build_ms`.
///
/// Shared by the binary and the Python bindings so both produce identical
/// snapshots from a loaded dataset.
pub fn build_snapshot(
    conn: &Connection,
    source: &str,
    lat_col: &str,
    lon_col: &str,
    row_count: i64,
    index_build_ms: u128,
) -> Result<Snapshot, AppError> {
    let info = crate::db::get_table_info_relation(conn, "data")?;
    let (spatial_bbox, spatial_count) = compute_extent(conn, lat_col, lon_col)?;

    let schema = TableSchema {
        source: source.to_string(),
        row_count,
        lat_col: lat_col.to_string(),
        lon_col: lon_col.to_string(),
        columns: info
            .col_names
            .iter()
            .zip(info.col_types.iter())
            .map(|(name, dtype)| ColumnMeta {
                name: name.clone(),
                dtype: dtype.clone(),
            })
            .collect(),
    };

    Ok(Snapshot {
        schema,
        index_build_ms,
        spatial_bbox,
        spatial_count,
    })
}

/// Compute the lat/lon bounding box and the count of spatially-valid rows from
/// the live `raw_data` table.
fn compute_extent(
    conn: &Connection,
    lat_col: &str,
    lon_col: &str,
) -> Result<(Option<BBox>, i64), AppError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT MIN(\"{lat}\"), MIN(\"{lon}\"), MAX(\"{lat}\"), MAX(\"{lon}\"), COUNT(*) FROM raw_data WHERE \"{lat}\" IS NOT NULL AND \"{lon}\" IS NOT NULL",
        lat = lat_col,
        lon = lon_col,
    ))?;
    let result: (Option<f64>, Option<f64>, Option<f64>, Option<f64>, i64) =
        stmt.query_row([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })?;
    let bbox = match result {
        (Some(min_lat), Some(min_lon), Some(max_lat), Some(max_lon), _) => {
            Some((min_lat, min_lon, max_lat, max_lon))
        }
        _ => None,
    };
    Ok((bbox, result.4))
}

/// Bind `addr`, serve the router built from `state`, and run until `shutdown`
/// resolves. Owns the axum + tokio I/O glue so callers (the binary and the Python
/// bindings) supply only a state and a shutdown signal — the async runtime stays
/// entirely on the caller's side.
pub async fn serve<A: tokio::net::ToSocketAddrs>(
    state: AppState,
    addr: A,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
}
