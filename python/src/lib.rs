//! Python bindings for Terrana.
//!
//! **Skeleton stage:** this currently exposes a single `ping()` smoke-test so the
//! full build stack (uv → maturin → pyo3/abi3 → the `terrana` crate → bundled
//! DuckDB) can be validated end to end before the real bindings are written.

use pyo3::prelude::*;

/// Smoke test. Forces the whole Rust stack (the `terrana` crate and its bundled
/// DuckDB) to link, and confirms a DuckDB connection can be created from inside
/// the extension module. Returns `"ok"` on success.
#[pyfunction]
fn ping() -> PyResult<String> {
    terrana_core::db::create_connection()
        .map(|_| "ok".to_string())
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))
}

/// The `terrana` Python module.
#[pymodule]
fn terrana(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(ping, m)?)?;
    Ok(())
}
