pub mod middleware;

use crate::config::Config;
use crate::handlers;
use crate::index::SpatialPoint;
use crate::store::DataTable;
use axum::Router;
use rstar::RTree;
use serde::Serialize;
use std::sync::Arc;
use std::time::Instant;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

#[derive(Clone)]
#[allow(dead_code)]
pub struct AppState {
    pub config: Arc<Config>,
    pub table: Arc<DataTable>,
    pub index: Arc<RTree<SpatialPoint>>,
    pub schema: Arc<TableSchema>,
    pub start_time: Instant,
    pub index_build_ms: u128,
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
