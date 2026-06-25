//! Python bindings for Terrana.
//!
//! Exposes Terrana's spatial query engine and geodesic geometry to Python. The
//! Rust engine is imported as `terrana_core` — it can't be `terrana`, because that
//! name belongs to the `#[pymodule] fn terrana` that defines this module, and a
//! local item shadows a same-named dependency crate.

use duckdb::Connection;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pythonize::pythonize;
use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;
use terrana_core::config::Config;
use terrana_core::db::query;
use terrana_core::server::{self, AppState};

/// Map any displayable error (Terrana's `AppError`, pythonize failures) to a
/// Python `RuntimeError`.
fn pyerr<E: std::fmt::Display>(e: E) -> PyErr {
    PyRuntimeError::new_err(e.to_string())
}

/// Parse a GeoJSON string into geo polygons. Accepts a bare Geometry, a Feature,
/// or a FeatureCollection, and flattens any MultiPolygons.
fn polygons_from_geojson(s: &str) -> PyResult<Vec<geo_types::Polygon<f64>>> {
    let gj: geojson::GeoJson = s
        .parse()
        .map_err(|e| pyerr(format!("Invalid GeoJSON: {e}")))?;
    let mut polys = Vec::new();
    match gj {
        geojson::GeoJson::Geometry(g) => collect_polygons(g, &mut polys)?,
        geojson::GeoJson::Feature(f) => {
            if let Some(g) = f.geometry {
                collect_polygons(g, &mut polys)?;
            }
        }
        geojson::GeoJson::FeatureCollection(fc) => {
            for f in fc.features {
                if let Some(g) = f.geometry {
                    collect_polygons(g, &mut polys)?;
                }
            }
        }
    }
    if polys.is_empty() {
        return Err(pyerr("No polygons found in input"));
    }
    Ok(polys)
}

fn collect_polygons(g: geojson::Geometry, out: &mut Vec<geo_types::Polygon<f64>>) -> PyResult<()> {
    let geom: geo_types::Geometry<f64> = g.try_into().map_err(|e| pyerr(format!("{e}")))?;
    match geom {
        geo_types::Geometry::Polygon(p) => out.push(p),
        geo_types::Geometry::MultiPolygon(mp) => out.extend(mp.0),
        _ => return Err(pyerr("Expected Polygon or MultiPolygon")),
    }
    Ok(())
}

/// Parse a GeoJSON string into geo points. Accepts a bare Geometry, a Feature, or
/// a FeatureCollection, flattens MultiPoints, and ignores non-point geometries.
fn points_from_geojson(s: &str) -> PyResult<Vec<geo_types::Point<f64>>> {
    let gj: geojson::GeoJson = s
        .parse()
        .map_err(|e| pyerr(format!("Invalid GeoJSON: {e}")))?;
    let mut pts = Vec::new();
    match gj {
        geojson::GeoJson::Geometry(g) => collect_points(g, &mut pts),
        geojson::GeoJson::Feature(f) => {
            if let Some(g) = f.geometry {
                collect_points(g, &mut pts);
            }
        }
        geojson::GeoJson::FeatureCollection(fc) => {
            for f in fc.features {
                if let Some(g) = f.geometry {
                    collect_points(g, &mut pts);
                }
            }
        }
    }
    Ok(pts)
}

fn collect_points(g: geojson::Geometry, out: &mut Vec<geo_types::Point<f64>>) {
    if let Ok(geom) = TryInto::<geo_types::Geometry<f64>>::try_into(g) {
        match geom {
            geo_types::Geometry::Point(p) => out.push(p),
            geo_types::Geometry::MultiPoint(mp) => out.extend(mp.0),
            _ => {}
        }
    }
}

/// Serialize a geo geometry plus properties into a GeoJSON Feature string.
fn feature_json(
    geometry: geojson::Geometry,
    properties: serde_json::Map<String, serde_json::Value>,
) -> String {
    geojson::Feature {
        bbox: None,
        geometry: Some(geometry),
        id: None,
        properties: Some(properties),
        foreign_members: None,
    }
    .to_string()
}

/// An open dataset: an in-memory DuckDB connection with the spatial R-tree index
/// built. Create one with [`load_csv`], [`load_parquet`], or [`load_geojson`].
#[pyclass]
struct TerranaSession {
    db: Arc<Mutex<Connection>>,
    #[pyo3(get)]
    lat_col: String,
    #[pyo3(get)]
    lon_col: String,
    #[pyo3(get)]
    source: String,
    #[pyo3(get)]
    row_count: i64,
}

/// Open `path`, ingest it, and build the spatial index. Extension determines the
/// format (CSV / Parquet / GeoJSON).
fn open_session(path: &str) -> PyResult<TerranaSession> {
    let conn = terrana_core::db::create_connection().map_err(pyerr)?;
    let info =
        terrana_core::ingest_file(&conn, Path::new(path), None, None, None).map_err(pyerr)?;
    Ok(TerranaSession {
        db: Arc::new(Mutex::new(conn)),
        lat_col: info.lat_col,
        lon_col: info.lon_col,
        source: path.to_string(),
        row_count: info.row_count,
    })
}

#[pymethods]
impl TerranaSession {
    /// Rows within `radius_m` metres of (`lat`, `lon`), nearest first. Each row is
    /// a dict with an added `_distance_km` key.
    fn query_radius<'py>(
        &self,
        py: Python<'py>,
        lat: f64,
        lon: f64,
        radius_m: f64,
    ) -> PyResult<Bound<'py, PyAny>> {
        let spatial = query::radius_filter(lat, lon, radius_m);
        let extra = query::distance_select(lat, lon);
        let rows = query::query(
            &self.db,
            Some(&spatial),
            &[],
            None,
            None,
            None,
            query::MAX_RESULT_LIMIT,
            Some(&extra),
            Some("_distance_km ASC"),
        )
        .map_err(pyerr)?;
        pythonize(py, &rows).map_err(pyerr)
    }

    /// Rows whose point lies within the bounding box. Each row is a dict.
    fn query_bbox<'py>(
        &self,
        py: Python<'py>,
        min_lat: f64,
        min_lon: f64,
        max_lat: f64,
        max_lon: f64,
    ) -> PyResult<Bound<'py, PyAny>> {
        let spatial = query::bbox_filter(min_lat, min_lon, max_lat, max_lon);
        let rows = query::query(
            &self.db,
            Some(&spatial),
            &[],
            None,
            None,
            None,
            query::MAX_RESULT_LIMIT,
            None,
            None,
        )
        .map_err(pyerr)?;
        pythonize(py, &rows).map_err(pyerr)
    }

    /// The `k` nearest rows to (`lat`, `lon`), nearest first. Each row is a dict
    /// with an added `_distance_km` key.
    ///
    /// Uses `geom IS NOT NULL` as the spatial predicate so the query runs against
    /// `raw_data` (which has the `geom` column the distance select needs).
    fn query_nearest<'py>(
        &self,
        py: Python<'py>,
        lat: f64,
        lon: f64,
        k: usize,
    ) -> PyResult<Bound<'py, PyAny>> {
        let extra = query::distance_select(lat, lon);
        let rows = query::query(
            &self.db,
            Some("geom IS NOT NULL"),
            &[],
            None,
            None,
            None,
            k,
            Some(&extra),
            Some("_distance_km ASC"),
        )
        .map_err(pyerr)?;
        pythonize(py, &rows).map_err(pyerr)
    }

    /// Start the embedded HTTP server on `bind:port` and **block** until interrupted
    /// (Ctrl-C). The tokio runtime lives entirely inside this call, and the GIL is
    /// released while it runs so other Python threads keep working.
    #[pyo3(signature = (port = 8080, bind = "127.0.0.1".to_string()))]
    fn serve(&self, py: Python<'_>, port: u16, bind: String) -> PyResult<()> {
        let state = self.build_state(port, &bind)?;
        let addr = format!("{}:{}", bind, port);
        let result: std::io::Result<()> = py.allow_threads(move || {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(async move {
                let shutdown = async {
                    let _ = tokio::signal::ctrl_c().await;
                };
                server::serve(state, addr, shutdown).await
            })
        });
        result.map_err(pyerr)
    }

    /// Start the embedded HTTP server on a background thread and return immediately.
    /// The returned [`TerranaServer`] stops it via `shutdown()` or context-manager exit.
    #[pyo3(signature = (port = 8080, bind = "127.0.0.1".to_string()))]
    fn serve_background(&self, port: u16, bind: String) -> PyResult<TerranaServer> {
        let state = self.build_state(port, &bind)?;
        let addr = format!("{}:{}", bind, port);
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let handle = std::thread::spawn(move || {
            let rt = match tokio::runtime::Runtime::new() {
                Ok(rt) => rt,
                Err(e) => {
                    eprintln!("terrana: failed to start tokio runtime: {e}");
                    return;
                }
            };
            rt.block_on(async move {
                let shutdown = async move {
                    let _ = rx.await;
                };
                if let Err(e) = server::serve(state, addr, shutdown).await {
                    eprintln!("terrana: server error: {e}");
                }
            });
        });
        Ok(TerranaServer {
            tx: Some(tx),
            handle: Some(handle),
        })
    }
}

impl TerranaSession {
    /// Assemble an [`AppState`] for the embedded server from this already-ingested
    /// session — no re-ingestion, just a snapshot built from the live connection.
    fn build_state(&self, port: u16, bind: &str) -> PyResult<AppState> {
        let snapshot = {
            let conn = self
                .db
                .lock()
                .map_err(|_| pyerr("database lock poisoned"))?;
            server::build_snapshot(
                &conn,
                &self.source,
                &self.lat_col,
                &self.lon_col,
                self.row_count,
                0,
            )
            .map_err(pyerr)?
        };
        let config = Config {
            file: std::path::PathBuf::from(&self.source),
            lat_col: Some(self.lat_col.clone()),
            lon_col: Some(self.lon_col.clone()),
            table: None,
            port,
            bind: bind.to_string(),
            watch: false,
            disk: false,
        };
        Ok(AppState {
            config: Arc::new(config),
            db: self.db.clone(),
            snapshot: Arc::new(RwLock::new(Arc::new(snapshot))),
            start_time: Instant::now(),
        })
    }
}

/// A handle to a server started by [`TerranaSession::serve_background`]. Stop it
/// with [`shutdown`](TerranaServer::shutdown), or use it as a context manager —
/// it shuts down on `__exit__`.
#[pyclass]
struct TerranaServer {
    tx: Option<tokio::sync::oneshot::Sender<()>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

#[pymethods]
impl TerranaServer {
    /// Signal the background server to stop and block until its thread has fully
    /// joined (the GIL is released while joining). Idempotent.
    fn shutdown(&mut self, py: Python<'_>) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.handle.take() {
            py.allow_threads(move || {
                let _ = handle.join();
            });
        }
    }

    fn __enter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    #[pyo3(signature = (_exc_type = None, _exc_value = None, _traceback = None))]
    fn __exit__(
        &mut self,
        py: Python<'_>,
        _exc_type: Option<Bound<'_, PyAny>>,
        _exc_value: Option<Bound<'_, PyAny>>,
        _traceback: Option<Bound<'_, PyAny>>,
    ) -> bool {
        self.shutdown(py);
        false
    }
}

impl Drop for TerranaServer {
    /// Stop the server if neither `shutdown()` nor the context manager already did,
    /// so a dropped handle never leaks the background thread. A plain join (the GIL
    /// may be held during drop) is safe here because the server thread runs only
    /// Rust (tokio / axum / DuckDB) and never acquires the GIL. No-op after an
    /// explicit `shutdown()`, which has already taken `tx` and `handle`.
    fn drop(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Load a CSV file and build its spatial index, returning an open session.
#[pyfunction]
fn load_csv(path: &str) -> PyResult<TerranaSession> {
    open_session(path)
}

/// Load a Parquet file and build its spatial index, returning an open session.
#[pyfunction]
fn load_parquet(path: &str) -> PyResult<TerranaSession> {
    open_session(path)
}

/// Load a GeoJSON file and build its spatial index, returning an open session.
#[pyfunction]
fn load_geojson(path: &str) -> PyResult<TerranaSession> {
    open_session(path)
}

/// Geodesic distance and bearing between two lon/lat points (WGS 84, ellipsoidal).
/// Returns a dict: `distance_m`, `distance_km`, `distance_mi`, `bearing_deg`.
#[pyfunction]
fn geodesic_distance<'py>(
    py: Python<'py>,
    lon1: f64,
    lat1: f64,
    lon2: f64,
    lat2: f64,
) -> PyResult<Bound<'py, PyAny>> {
    let from = geo_types::Point::new(lon1, lat1);
    let to = geo_types::Point::new(lon2, lat2);
    let result = terrana_core::geometry::measure::geodesic_distance(from, to);
    pythonize(py, &result).map_err(pyerr)
}

/// Geodesic area + perimeter of a GeoJSON Polygon/MultiPolygon (or a Feature /
/// FeatureCollection of them). Returns a dict: `area_m2`, `area_km2`, `area_ha`,
/// `area_acres`, `perimeter_m`.
#[pyfunction]
fn geodesic_area<'py>(py: Python<'py>, geojson_str: &str) -> PyResult<Bound<'py, PyAny>> {
    let polys = polygons_from_geojson(geojson_str)?;
    let result = terrana_core::geometry::area::compute_area(&polys);
    pythonize(py, &result).map_err(pyerr)
}

/// Convex hull of the points in a GeoJSON input. Returns a GeoJSON Feature string
/// whose geometry is the hull and whose properties carry the geodesic `area_m2`,
/// `area_km2`, `area_ha`, `perimeter_m`, and `point_count`.
#[pyfunction]
fn convex_hull(geojson_str: &str) -> PyResult<String> {
    let points = points_from_geojson(geojson_str)?;
    if points.len() < 3 {
        return Err(pyerr("Need at least 3 points for convex hull"));
    }
    let result = terrana_core::geometry::hull::compute_convex_hull(&points);
    let mut props = serde_json::Map::new();
    props.insert("area_m2".to_string(), result.area_m2.into());
    props.insert("area_km2".to_string(), result.area_km2.into());
    props.insert("area_ha".to_string(), result.area_ha.into());
    props.insert("perimeter_m".to_string(), result.perimeter_m.into());
    props.insert("point_count".to_string(), result.point_count.into());
    Ok(feature_json((&result.hull).into(), props))
}

/// Geodesic buffer of `segments` vertices around (`lon`, `lat`) at `distance_m`
/// metres. Returns a GeoJSON Feature string (the buffer polygon) with `area_m2`,
/// `area_km2`, `distance_m`, and `segments` properties.
#[pyfunction]
#[pyo3(signature = (lon, lat, distance_m, segments = 64))]
fn buffer(lon: f64, lat: f64, distance_m: f64, segments: usize) -> PyResult<String> {
    let poly = terrana_core::geometry::buffer::compute_buffer(
        geo_types::Point::new(lon, lat),
        distance_m,
        segments,
    );
    let area = terrana_core::geometry::area::compute_area(std::slice::from_ref(&poly));
    let mut props = serde_json::Map::new();
    props.insert("area_m2".to_string(), area.area_m2.into());
    props.insert("area_km2".to_string(), area.area_km2.into());
    props.insert("distance_m".to_string(), distance_m.into());
    props.insert("segments".to_string(), segments.into());
    Ok(feature_json((&poly).into(), props))
}

/// The `terrana` Python module.
#[pymodule]
fn terrana(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<TerranaSession>()?;
    m.add_class::<TerranaServer>()?;
    m.add_function(wrap_pyfunction!(load_csv, m)?)?;
    m.add_function(wrap_pyfunction!(load_parquet, m)?)?;
    m.add_function(wrap_pyfunction!(load_geojson, m)?)?;
    m.add_function(wrap_pyfunction!(geodesic_distance, m)?)?;
    m.add_function(wrap_pyfunction!(geodesic_area, m)?)?;
    m.add_function(wrap_pyfunction!(convex_hull, m)?)?;
    m.add_function(wrap_pyfunction!(buffer, m)?)?;
    Ok(())
}
