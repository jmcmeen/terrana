//! Command-line interface for the `terrana` server.
//!
//! This is the single implementation of the CLI, shared by two front ends:
//!
//! * the `terrana` binary ([`src/main.rs`](../main.rs.html) is a thin shell over it), and
//! * the `terrana` Python console script (`terrana:_run_cli` in the `terrana-py`
//!   crate, installed by `pip install terrana`).
//!
//! Both call [`run`] with an explicit argv, so `terrana serve …` behaves identically
//! whether it came from a release binary, `cargo run`, or a pip-installed wheel.
//!
//! Only built with the `server` feature (it needs axum/tokio); see [`crate`] docs.

use crate::config::Config;
use crate::db;
use crate::error::AppError;
use crate::server::{self, AppState, Snapshot};
use clap::{Parser, Subcommand};
use duckdb::Connection;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "terrana", version, about = "Zero-config spatial API server")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Start the spatial API server for a data file
    Serve {
        /// Path to CSV, Parquet, GeoJSON, or .duckdb file
        file: PathBuf,

        /// Latitude column name [auto-detected if omitted]
        #[arg(long)]
        lat: Option<String>,

        /// Longitude column name [auto-detected if omitted]
        #[arg(long)]
        lon: Option<String>,

        /// Table name (DuckDB files only)
        #[arg(long)]
        table: Option<String>,

        /// HTTP port
        #[arg(long, default_value = "8080")]
        port: u16,

        /// Bind address
        #[arg(long, default_value = "127.0.0.1")]
        bind: String,

        /// Re-index when source file changes
        #[arg(long)]
        watch: bool,

        /// Use on-disk DuckDB storage instead of in-memory (reduces RAM usage for large files)
        #[arg(long)]
        disk: bool,
    },
}

/// Parse `args` as the `terrana` CLI and run the requested command to completion.
///
/// Builds its own multi-threaded tokio runtime and blocks until the process is
/// terminated — the server runs until killed (`Ctrl-C` / `SIGTERM`), matching the
/// standalone binary. Callers pass argv explicitly (`std::env::args_os()` for the
/// binary, `sys.argv` for the Python console script) so one code path backs both.
pub fn run<I, T>(args: I) -> anyhow::Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    init_tracing();
    let cli = Cli::parse_from(args);
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
        } => serve(file, lat, lon, table, port, bind, watch, disk),
    }
}

/// Install the global tracing subscriber once. Idempotent: invoking the CLI a second
/// time in the same process (possible from the Python module) is a harmless no-op.
fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("terrana=info")),
        )
        .try_init();
}

/// Ingest `file`, build the spatial index, and serve the REST API until killed.
#[allow(clippy::too_many_arguments)]
fn serve(
    file: PathBuf,
    lat: Option<String>,
    lon: Option<String>,
    table: Option<String>,
    port: u16,
    bind: String,
    watch: bool,
    disk: bool,
) -> anyhow::Result<()> {
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

    // Hold _tmp_dir in scope so the temp directory lives as long as the server.
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

    let addr = format!("{}:{}", bind, port);
    info!("listening on {}", addr);

    // Build the runtime here (rather than via `#[tokio::main]`) so this same fn can
    // be driven from the Python console script, which is not an async context.
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async move {
        // A never-resolving shutdown preserves run-until-killed behaviour, shared by
        // the binary and the Python console script.
        server::serve(state, addr, std::future::pending::<()>()).await
    })?;

    Ok(())
}

/// Ingest the source file into `conn`, build the spatial index, and assemble a
/// [`Snapshot`]. Used both at startup and on every `--watch` reload.
///
/// The work is ordered **stage → validate → promote** so a reload is failure-atomic:
/// the file is read into a staging table and its lat/lon columns are validated *before*
/// the live dataset is dropped. Any failure in that risky phase returns `Err` with the
/// previous dataset left intact, so `--watch` keeps serving the old data on a bad/
/// half-written file instead of blanking out.
fn build_snapshot(
    conn: &Connection,
    source: &str,
    abs_path: &Path,
    table: Option<&str>,
    lat_override: Option<&str>,
    lon_override: Option<&str>,
) -> Result<Snapshot, AppError> {
    // 1. Risky external read into a staging table — the live dataset is untouched on failure.
    db::loader::stage_file(conn, abs_path, table)?;

    // 2. Validate the new file's columns before committing to the swap. On failure,
    //    discard the stage and bail so a bad reload keeps serving the old data.
    let staged = db::get_table_info_relation(conn, "raw_data_stage")?;
    let (lat_col, lon_col) =
        match db::loader::detect_lat_lon(&staged.col_names, lat_override, lon_override) {
            Ok(cols) => cols,
            Err(e) => {
                let _ = db::loader::discard_stage(conn);
                return Err(e);
            }
        };
    info!(lat = %lat_col, lon = %lon_col, "columns detected");

    // 3. Commit: drop the previous dataset and promote the staged tables to live.
    db::loader::promote_stage(conn)?;

    // 4. Build the geometry column + R-tree index (operates on in-DB data; reliable).
    let start_build = Instant::now();
    db::loader::add_spatial_index(conn, &lat_col, &lon_col)?;
    let index_build_ms = start_build.elapsed().as_millis();
    info!(ms = %index_build_ms, "spatial index built");

    // Assemble the snapshot (extent + schema) via the shared library helper.
    server::build_snapshot(
        conn,
        source,
        &lat_col,
        &lon_col,
        staged.row_count,
        index_build_ms,
    )
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
