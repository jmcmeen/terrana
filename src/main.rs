mod cli;
mod config;
mod db;
mod error;
mod geometry;
mod handlers;
mod index;
mod output;
mod server;

use clap::Parser;
use cli::{Cli, Commands};
use config::Config;
use server::{AppState, ColumnMeta, TableSchema};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("terrana=info")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Serve {
            file,
            lat,
            lon,
            table,
            port,
            bind,
            watch,
        } => {
            let config = Config {
                file: file.clone(),
                lat_col: lat.clone(),
                lon_col: lon.clone(),
                table: table.clone(),
                port,
                bind: bind.clone(),
                watch,
            };

            info!("terrana v{}", env!("CARGO_PKG_VERSION"));
            info!("source: {}", file.display());

            // Verify file exists
            if !file.exists() {
                anyhow::bail!("File not found: {}", file.display());
            }

            // Set up DuckDB
            let conn = db::create_connection()?;
            let abs_path = std::fs::canonicalize(&file)?;
            db::loader::ingest_file(&conn, &abs_path, table.as_deref())?;

            // Detect lat/lon columns
            let (lat_col, lon_col) =
                db::loader::detect_lat_lon(&conn, lat.as_deref(), lon.as_deref())?;
            info!(lat = %lat_col, lon = %lon_col, "columns detected");

            // Get schema info
            let columns_meta = db::query::get_columns(&conn)?;
            let row_count = db::query::row_count(&conn)?;
            info!(rows = row_count, "data loaded");

            // Build R-tree index
            let start_build = Instant::now();
            let tree = index::build::build_rtree(&conn, &lat_col, &lon_col)?;
            let index_build_ms = start_build.elapsed().as_millis();

            let schema = TableSchema {
                source: file.display().to_string(),
                row_count,
                lat_col,
                lon_col,
                columns: columns_meta
                    .into_iter()
                    .map(|(name, dtype)| ColumnMeta { name, dtype })
                    .collect(),
            };

            let state = AppState {
                config: Arc::new(config),
                db: Arc::new(Mutex::new(conn)),
                index: Arc::new(tree),
                schema: Arc::new(schema),
                start_time: Instant::now(),
                index_build_ms,
            };

            let app = server::build_router(state);

            let addr = format!("{}:{}", bind, port);
            info!("listening on {}", addr);

            let listener = tokio::net::TcpListener::bind(&addr).await?;
            axum::serve(listener, app).await?;
        }
    }

    Ok(())
}
