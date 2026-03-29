mod cli;
mod config;
mod db;
mod error;
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
            disk,
        } => {
            let config = Config {
                file: file.clone(),
                lat_col: lat.clone(),
                lon_col: lon.clone(),
                table: table.clone(),
                port,
                bind: bind.clone(),
                watch,
                disk,
            };

            info!("terrana v{}", env!("CARGO_PKG_VERSION"));
            info!("source: {}", file.display());

            if !file.exists() {
                anyhow::bail!("File not found: {}", file.display());
            }

            // Create DuckDB connection and load file
            let abs_path = std::fs::canonicalize(&file)?;

            // Hold _tmp_dir in scope so the temp directory lives as long as the server
            let _tmp_dir;
            let conn = if disk {
                info!("using on-disk DuckDB storage");
                let (c, td) = db::create_disk_connection()?;
                _tmp_dir = Some(td);
                c
            } else {
                _tmp_dir = None;
                db::create_connection()?
            };
            db::loader::load_file(&conn, &abs_path, table.as_deref())?;

            let db_mutex = Arc::new(Mutex::new(conn));

            // Get schema info
            let table_info = db::get_table_info(&db_mutex)?;

            // Detect lat/lon columns
            let (lat_col, lon_col) =
                db::loader::detect_lat_lon(&table_info.col_names, lat.as_deref(), lon.as_deref())?;
            info!(lat = %lat_col, lon = %lon_col, "columns detected");

            // Build R-tree index
            let start_build = Instant::now();
            let tree = index::build::build_rtree(&db_mutex, &lat_col, &lon_col)?;
            let index_build_ms = start_build.elapsed().as_millis();

            let schema = TableSchema {
                source: file.display().to_string(),
                row_count: table_info.row_count,
                lat_col,
                lon_col,
                columns: table_info
                    .col_names
                    .iter()
                    .zip(table_info.col_types.iter())
                    .map(|(name, dtype)| ColumnMeta {
                        name: name.clone(),
                        dtype: dtype.clone(),
                    })
                    .collect(),
            };

            let state = AppState {
                config: Arc::new(config),
                db: db_mutex,
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
