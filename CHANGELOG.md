# Changelog

All notable changes to Terrana will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-06-02

### Added

- CLI with `terrana serve` subcommand and `--lat`, `--lon`, `--port`, `--bind`, `--watch`, `--disk` options
- Auto-detection of lat/lon columns from common naming conventions
- File ingestion for CSV, Parquet, GeoJSON, and DuckDB files
- DuckDB spatial extension R-tree index on geometry column for accelerated spatial queries
- `--disk` flag for on-disk DuckDB storage — required for large datasets (250M+ rows) that exceed available RAM
- `GET /query` endpoint with radius, bounding box, and nearest neighbor modes
- `POST /query/within` for point-in-polygon queries via `ST_Contains` (R-tree accelerated)
- `select=`, `where=`, `group_by=`, `agg=`, `limit=` query parameters
- JSON, CSV, and GeoJSON output formats
- Geometry endpoints: area, convex-hull, centroid, buffer, dissolve, simplify, distance, bounds
- Geometry area/perimeter use geodesic algorithms (Karney, WGS 84 ellipsoid via `geo` crate)
- Query path distances (radius, nearest) use haversine via DuckDB `ST_Distance_Sphere`
- `GET /health`, `GET /schema`, `GET /stats` metadata endpoints
- CORS support and request tracing via tower-http
- Tracing/logging with `RUST_LOG` env filter
- GitHub Actions CI (check, clippy, fmt, cross-platform build) and release workflows
- Dockerfile and docker-compose.yml for containerized deployment
- Benchmark script (`bench.sh`) and data generators for 10K–250M row datasets
- `--watch` now re-ingests the source file and atomically swaps the served dataset on change (previously the flag was accepted but had no effect)
- Test suite: unit tests for validation, SQL builders, lat/lon detection, and geodesic area; integration tests in `tests/api.rs` exercising the live HTTP API (run with `cargo test -- --include-ignored`)
- crates.io package metadata (description, license, repository, keywords, categories, MSRV) and dual `MIT OR Apache-2.0` licensing
- Community files: `CONTRIBUTING.md`, `SECURITY.md`, GitHub issue/PR templates, `.editorconfig`, `rust-toolchain.toml`
- `Makefile` with shortcuts for build/run/test/lint/package tasks (`make help`)
- "Installing Rust" guide in the README
- CI `test` job (runs unit + integration tests); clippy now lints all targets

### Changed

- `GET /stats` returns `null` for `bbox`/`centroid` when the dataset has no spatially-valid rows, instead of a fake `(0, 0)`

### Security

- Column name validation for all user-supplied column names, group_by, agg, and select params
- Fixed a SQL-injection vector in the `--table` argument for `.duckdb` sources: the table identifier is now validated and quoted before interpolation
- Bounding-box query parameters are now range-validated (lat ∈ [-90, 90], lon ∈ [-180, 180], min ≤ max)
