//! The resolved runtime configuration, built from parsed CLI args and shared through
//! [`AppState`](crate::server::AppState) behind an `Arc`.

use std::path::PathBuf;

// Fields are retained for introspection and future handler use; not all are read today.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Config {
    pub file: PathBuf,
    pub lat_col: Option<String>,
    pub lon_col: Option<String>,
    pub table: Option<String>,
    pub port: u16,
    pub bind: String,
    pub watch: bool,
    pub disk: bool,
}
