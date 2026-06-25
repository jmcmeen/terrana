//! Terrana — zero-config spatial queries and geodesic geometry.
//!
//! Point Terrana at a CSV, Parquet, GeoJSON, or DuckDB file with lat/lon columns
//! and run spatial queries and geodesic geometry operations against it — no
//! database setup required.
//!
//! # Two ways to use the crate
//!
//! **As a library** (no HTTP server). Load a file with [`ingest_file`], then run
//! SQL-backed spatial queries via [`db::query`] or geodesic computations via
//! [`geometry`]:
//!
//! ```no_run
//! use terrana::{db, ingest_file};
//!
//! let conn = db::create_connection()?;
//! let info = ingest_file(&conn, std::path::Path::new("observations.csv"), None, None, None)?;
//! println!("loaded {} rows; lat={}, lon={}", info.row_count, info.lat_col, info.lon_col);
//! # Ok::<(), terrana::AppError>(())
//! ```
//!
//! **As an HTTP server** (the `server` feature, enabled by default). The binary
//! and the [`server`] module assemble an [`axum`](https://docs.rs/axum) router
//! over the same primitives. Disable default features to depend on the pure
//! library without pulling in axum/tokio:
//!
//! ```toml
//! terrana = { version = "0.2", default-features = false }
//! ```
//!
//! # Geodesic rules
//!
//! Area, perimeter, explicit distance, and buffer geometry use geodesic
//! algorithms on the WGS 84 ellipsoid (Karney / Vincenty via the `geo` crate),
//! never planar math. See [`geometry`].

// Pure library — always available, no HTTP dependencies.
pub mod config;
pub mod db;
pub mod error;
pub mod geometry;

// HTTP server layer — gated so library-only consumers avoid axum/tokio/tower.
#[cfg(feature = "server")]
pub mod cli;
#[cfg(feature = "server")]
pub mod handlers;
#[cfg(feature = "server")]
pub mod output;
#[cfg(feature = "server")]
pub mod server;

// The most useful items re-exported at the crate root.
pub use db::loader::{detect_lat_lon, ingest_file, IngestInfo};
pub use db::query::query;
pub use error::AppError;
