# Changelog

All notable changes to Terrana will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.1] - 2026-06-25

### Added

- **`terrana` command line in the Python package.** `pip install terrana` now installs a `terrana` console command that runs the exact same server as the standalone Rust binary — `terrana serve <file> [--lat … --lon … --table … --port … --bind … --watch --disk]`. The Python and Rust distributions now share one CLI, so `pip install terrana && terrana serve data.csv` works with no Rust toolchain. (The in-process library API and `serve_background`/`serve` are unchanged.)

### Changed

- The CLI (argument parsing + serve orchestration) moved out of the binary into the library as `terrana::cli::run`, now shared by the `terrana` binary and the Python console script. `clap` became part of the `server` feature, keeping the `default-features = false` pure-library build free of it.

### Fixed

- Python wheels: the PyPI publish workflow builds Linux wheels in a `manylinux_2_28` container (was the default `manylinux2014`), whose C++ toolchain is new enough to compile the bundled DuckDB engine — the manylinux2014 build failed in the `libduckdb-sys` build script.

## [0.2.0] - 2026-06-25

### Added

- **Library crate.** Terrana is now a `lib + bin` crate. `src/lib.rs` exposes a public API so the engine can be used in-process without the HTTP server: `ingest_file`, `detect_lat_lon`, `query`, `AppError`, and the geodesic `geometry` modules.
- `db::loader::ingest_file` — load a file end to end (stage → detect lat/lon → promote → build the R-tree index) in one call, returning `IngestInfo { lat_col, lon_col, row_count }`.
- `server` Cargo feature, enabled by default. Depend on Terrana with `default-features = false` to get the pure library without pulling in axum / tokio / tower.
- **Python bindings** (`terrana` on PyPI, built with PyO3 + maturin). A single stable-ABI (`abi3`) wheel for CPython 3.9+ exposing both an in-process library mode — `load_csv`/`load_parquet`/`load_geojson`, `query_radius`/`query_bbox`/`query_nearest`, `geodesic_distance`/`geodesic_area`, `convex_hull`, `buffer` — and an embedded HTTP server managed from Python (`serve_background`/`serve`/`shutdown`, context manager). The tokio runtime runs entirely in a Rust thread, off the GIL.

### Changed

- Geodesic math moved out of the Axum handlers into pure `terrana::geometry` modules (`area`, `buffer`, `hull`, `dissolve`, `simplify`, `measure`). Handlers are now thin glue; HTTP responses are unchanged.

### Fixed

- `POST /geometry/buffer` reported the area as ~510,000,000 km² (the Earth's entire surface): the ring was wound clockwise, so `geodesic_area_unsigned` measured its complement. The ring is now generated counter-clockwise (GeoJSON right-hand rule), so the reported area is the buffer disk as expected.

## [0.1.1] - 2026-06-02

### Fixed

- `--watch` reloads no longer blank out the dataset when the source file is malformed or half-written mid-save. The new file is now staged and its lat/lon columns validated *before* the live dataset is replaced, so a failed reload leaves the previous data serving and the watcher recovers on the next good write.
- `testdata/bench.sh` no longer exits non-zero (Error 143) when its background server is terminated on an otherwise successful run, so `make bench` reports success correctly. Genuine failures still propagate.
- Packaging: `testdata/generate.py` is no longer published to crates.io — the `exclude` glob missed the renamed generator, so benchmark/dev tooling was leaking into the package.

### Changed

- Consolidated the two benchmark data generators into a single `testdata/generate.py` (`--preset bench` for 10K/100K/1M, `--preset 250m` for the 250M-row set); removed `generate_benchdata.py` and `generate_250m.py`.
- Moved `bench.sh` into `testdata/` (now runnable from any directory) and added `make gen`, `make gen-250m`, and `make bench` targets.

### Added

- Integration tests for `--watch` reload: new rows are reflected, and the old dataset is preserved when a reload hits a bad file.

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
