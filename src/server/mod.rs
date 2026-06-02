//! Axum router assembly and the shared [`AppState`] passed to every handler.

pub mod middleware;

use crate::config::Config;
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
