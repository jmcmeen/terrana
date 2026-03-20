use std::path::PathBuf;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Config {
    pub file: PathBuf,
    pub lat_col: Option<String>,
    pub lon_col: Option<String>,
    pub port: u16,
    pub bind: String,
    pub watch: bool,
}
