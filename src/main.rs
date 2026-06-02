//! Terrana — zero-config spatial API server.
//!
//! Entry point: parse the CLI, ingest the source file into DuckDB, build the spatial
//! index, and serve the REST API with axum. With `--watch`, a background thread
//! re-ingests the file and atomically swaps the served [`server::Snapshot`] on change.

mod cli;
mod config;
mod db;
mod error;
mod handlers;
mod output;
mod server;

use clap::Parser;
use cli::{Cli, Commands};
use config::Config;
use duckdb::Connection;
use error::AppError;
use server::{AppState, BBox, ColumnMeta, Snapshot, TableSchema};
use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};
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

            let abs_path = std::fs::canonicalize(&file)?;
            let source = file.display().to_string();

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

            // Ingest the file, build the spatial index, and compute the initial snapshot.
            let snapshot = build_snapshot(
                &conn,
                &source,
                &abs_path,
                table.as_deref(),
                lat.as_deref(),
                lon.as_deref(),
            )?;

            let db_mutex = Arc::new(Mutex::new(conn));
            let snapshot = Arc::new(RwLock::new(Arc::new(snapshot)));

            if watch {
                spawn_watcher(
                    abs_path,
                    source,
                    table.clone(),
                    lat.clone(),
                    lon.clone(),
                    db_mutex.clone(),
                    snapshot.clone(),
                );
            }

            let state = AppState {
                config: Arc::new(config),
                db: db_mutex,
                snapshot,
                start_time: Instant::now(),
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

/// Ingest the source file into `conn`, build the spatial index, and assemble a
/// [`Snapshot`]. Used both at startup and on every `--watch` reload, so it first
/// drops any existing dataset artifacts to start from a clean slate.
fn build_snapshot(
    conn: &Connection,
    source: &str,
    abs_path: &Path,
    table: Option<&str>,
    lat_override: Option<&str>,
    lon_override: Option<&str>,
) -> Result<Snapshot, AppError> {
    db::loader::drop_dataset(conn)?;
    db::loader::load_file(conn, abs_path, table)?;

    let table_info = db::get_table_info_conn(conn)?;
    let (lat_col, lon_col) =
        db::loader::detect_lat_lon(&table_info.col_names, lat_override, lon_override)?;
    info!(lat = %lat_col, lon = %lon_col, "columns detected");

    let start_build = Instant::now();
    db::loader::add_spatial_index(conn, &lat_col, &lon_col)?;
    let index_build_ms = start_build.elapsed().as_millis();
    info!(ms = %index_build_ms, "spatial index built");

    let (spatial_bbox, spatial_count) = compute_extent(conn, &lat_col, &lon_col)?;

    let schema = TableSchema {
        source: source.to_string(),
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

    Ok(Snapshot {
        schema,
        index_build_ms,
        spatial_bbox,
        spatial_count,
    })
}

/// Compute the lat/lon bounding box and the count of spatially-valid rows.
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

/// Spawn a background thread that watches `abs_path` and re-ingests it on change,
/// atomically swapping the shared snapshot. Watches the parent directory and filters
/// by path so it survives editors that replace files via atomic rename.
#[allow(clippy::too_many_arguments)]
fn spawn_watcher(
    abs_path: std::path::PathBuf,
    source: String,
    table: Option<String>,
    lat: Option<String>,
    lon: Option<String>,
    db: Arc<Mutex<Connection>>,
    snapshot: Arc<RwLock<Arc<Snapshot>>>,
) {
    use notify::{EventKind, RecursiveMode, Watcher};
    use std::sync::mpsc;
    use std::time::Duration;

    std::thread::spawn(move || {
        let (tx, rx) = mpsc::channel();
        let mut watcher = match notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        }) {
            Ok(w) => w,
            Err(e) => {
                tracing::error!("watch: failed to create watcher: {}", e);
                return;
            }
        };

        let watch_target = abs_path.parent().unwrap_or(&abs_path).to_path_buf();
        if let Err(e) = watcher.watch(&watch_target, RecursiveMode::NonRecursive) {
            tracing::error!("watch: failed to watch {}: {}", watch_target.display(), e);
            return;
        }
        info!("watching {} for changes", abs_path.display());

        let is_relevant = |res: &notify::Result<notify::Event>| -> bool {
            match res {
                Ok(ev) => {
                    matches!(ev.kind, EventKind::Modify(_) | EventKind::Create(_))
                        && ev.paths.iter().any(|p| p == &abs_path)
                }
                Err(e) => {
                    tracing::warn!("watch error: {}", e);
                    false
                }
            }
        };

        // Block for an event, then coalesce any burst within a short debounce window.
        while let Ok(first) = rx.recv() {
            let mut relevant = is_relevant(&first);
            while let Ok(ev) = rx.recv_timeout(Duration::from_millis(300)) {
                relevant |= is_relevant(&ev);
            }
            if !relevant {
                continue;
            }

            info!("source changed — reloading");
            let result = match db.lock() {
                Ok(conn) => build_snapshot(
                    &conn,
                    &source,
                    &abs_path,
                    table.as_deref(),
                    lat.as_deref(),
                    lon.as_deref(),
                ),
                Err(e) => {
                    tracing::error!("watch: db lock poisoned: {}", e);
                    continue;
                }
            };
            match result {
                Ok(new_snap) => {
                    if let Ok(mut guard) = snapshot.write() {
                        *guard = Arc::new(new_snap);
                    }
                    info!("reload complete");
                }
                Err(e) => tracing::error!("reload failed: {}", e),
            }
        }
    });
}
