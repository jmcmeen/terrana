//! Terrana — zero-config spatial API server.
//!
//! Thin CLI shell: it forwards the process arguments to [`terrana::cli::run`], which
//! parses the command, ingests the source file, builds the spatial index, and serves
//! the REST API with axum. All the real work lives in the [`terrana`] library so the
//! exact same CLI backs both this binary and the `terrana` Python console script
//! (`pip install terrana`).

fn main() -> anyhow::Result<()> {
    terrana::cli::run(std::env::args_os())
}
