mod cli;
mod config;
mod error;
mod handlers;
mod index;
mod output;
mod server;
mod store;

use clap::Parser;
use cli::{Cli, Commands};
use config::Config;
use server::{AppState, ColumnMeta, TableSchema};
use std::sync::Arc;
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
            port,
            bind,
            watch,
        } => {
            let config = Config {
                file: file.clone(),
                lat_col: lat.clone(),
                lon_col: lon.clone(),
                port,
                bind: bind.clone(),
                watch,
            };

            info!("terrana v{}", env!("CARGO_PKG_VERSION"));
            info!("source: {}", file.display());

            if !file.exists() {
                anyhow::bail!("File not found: {}", file.display());
            }

            // Load file into memory
            let abs_path = std::fs::canonicalize(&file)?;
            let data_table = store::loader::load_file(&abs_path)?;

            // Detect lat/lon columns
            let (lat_col, lon_col) =
                store::loader::detect_lat_lon(&data_table, lat.as_deref(), lon.as_deref())?;
            info!(lat = %lat_col, lon = %lon_col, "columns detected");

            // Build R-tree index
            let start_build = Instant::now();
            let tree = index::build::build_rtree(&data_table, &lat_col, &lon_col);
            let index_build_ms = start_build.elapsed().as_millis();

            let schema = TableSchema {
                source: file.display().to_string(),
                row_count: data_table.row_count,
                lat_col,
                lon_col,
                columns: data_table
                    .columns
                    .iter()
                    .map(|(name, dtype)| ColumnMeta {
                        name: name.clone(),
                        dtype: dtype.clone(),
                    })
                    .collect(),
            };

            let state = AppState {
                config: Arc::new(config),
                table: Arc::new(data_table),
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
