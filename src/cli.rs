use clap::{Parser, Subcommand};
use std::path::PathBuf;

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
        /// Path to CSV or GeoJSON file
        file: PathBuf,

        /// Latitude column name [auto-detected if omitted]
        #[arg(long)]
        lat: Option<String>,

        /// Longitude column name [auto-detected if omitted]
        #[arg(long)]
        lon: Option<String>,

        /// HTTP port
        #[arg(long, default_value = "8080")]
        port: u16,

        /// Bind address
        #[arg(long, default_value = "127.0.0.1")]
        bind: String,

        /// Re-index when source file changes
        #[arg(long)]
        watch: bool,
    },
}
