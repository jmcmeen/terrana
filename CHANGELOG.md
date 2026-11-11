# Changelog

All notable changes to Terrana will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.3] - 2026-11-11

### Added

- Agent skills in `skills/`: folders an AI agent such as Claude Code installs so it can drive
  terrana for a task, with decisions resting on scripts that measure and check. The first,
  `check-coordinates`, counts rows with no point, coordinates out of range, points at (0, 0)
  and points outside an expected area, and fails (exit 2) when the flagged share is above a
  threshold (an empty dataset fails too; a box may cross the antimeridian). Scripts start
  `terrana serve` on a file or use a running server (`TERRANA_URL`,
  `TERRANA_TOKEN`); `skills/README.md` gives the conventions, and each skill has an eval in
  `tests/skills/` that CI runs. In Claude Code, install them all as a plugin:
  `/plugin install terrana --marketplace jmcmeen/terrana` (#77)

## [0.3.2] - 2026-11-11

### Security

- The Python bindings use pyo3 0.29 (was 0.23), which fixes three pyo3 advisories: an
  out-of-bounds read in `PyList`/`PyTuple` iterators, a missing `Sync` bound on
  `PyCFunction::new_closure` closures, and a buffer overflow risk in `PyString::from_object`.
  terrana's own code doesn't call the affected APIs. pythonize moves to 0.29 with it. Building the
  Python package from source now needs Rust 1.83; the engine crate still builds on 1.82 (#53)

### Fixed

- Python: `query_radius`, `query_bbox` and `query_nearest` release the GIL while DuckDB runs, so a
  query waiting behind the background server no longer stalls every other Python thread (#73)
- Python: `TerranaServer.shutdown()` can be called from several threads at once; each call returns
  once the server has stopped, instead of the later ones raising `Already borrowed` (#74)

## [0.3.1] - 2026-11-11

> **Python users:** this is the first release on PyPI since 0.2.12 (0.3.0's wheels didn't
> reach PyPI). It includes 0.3.0's fix for **radius and nearest results, which were wrong in
> every earlier release**: if you used `query_radius()`, `query_nearest()` or `_distance_km`,
> rerun those queries. See the [0.3.0 release](https://github.com/jmcmeen/terrana/releases/tag/v0.3.0) and
> [#62](https://github.com/jmcmeen/terrana/issues/62).

### Fixed

- The macOS x86_64 wheel builds again, so every wheel reaches PyPI: 0.3.0's PyPI upload was skipped
  because that wheel failed. DuckDB 2.0 uses aligned deallocation, which needs macOS 10.13, and
  maturin's x86_64 default was 10.12. The macOS x86_64 wheel and binary now target 10.13 (arm64
  stays at 11.0), and CI builds that wheel too (#69)
- CI and release builds no longer fail when a restored Rust cache makes libduckdb-sys rerun its
  build script, for example after a toolchain or Cargo.lock change (#72)

### Security

- rustls updated to 0.23.46 for four advisories. It is used only by the test suite's HTTP client,
  so released binaries and wheels were not affected (#71)

## [0.3.0] - 2026-11-11

> **Radius and nearest results were wrong in every earlier release.** Distances were computed
> with latitude and longitude swapped, so `GET /query` with `radius=` or `nearest=`, the Python
> `query_radius()` and `query_nearest()`, and every `_distance_km` value returned the wrong rows
> and the wrong distances. For example, a 6.08 km radius on the bundled test data returned 14
> rows where 4 are within range. If you used radius or nearest results, or `_distance_km`, from
> an earlier version, rerun those queries on this release. Bounding-box (`bbox=`),
> point-in-polygon (`/query/within`) and `/geometry/*` results were not affected. Details:
> [#62](https://github.com/jmcmeen/terrana/issues/62).

### Added

- Release binaries for Linux arm64 (`aarch64-unknown-linux-gnu`) and Windows x86_64
  (`x86_64-pc-windows-msvc`, a `.zip`), and a manylinux 2_28 aarch64 wheel on PyPI. Both Linux
  arm64 builds run natively on GitHub's arm64 runners. CI now builds and tests on Linux x86_64
  and arm64, macOS and Windows (#59)

### Changed

- CI can be run by hand on any branch (`gh workflow run ci.yml --ref <branch>`), so stacked PRs
  above the bottom one get CI before they merge; such runs never release (#67)
- DuckDB 2.0 (`duckdb` 1.20000), in the crate and the Python bindings. It no longer builds with the
  `zip` crate, which had four open security advisories (one critical) that couldn't be fixed on
  DuckDB 1.x. The spatial extension is downloaded again for 2.0 on first run (#52)
- The Linux release binaries are built in the manylinux 2_28 image, like the wheels, so they need
  glibc 2.28+ (RHEL 8+, Debian 10+, Amazon Linux 2023) rather than the runner's 2.39. CI builds and
  tests each released binary the way `publish.yml` builds it, including the macOS x86_64 cross
  build (built, not run), and the Python bindings on arm64 Linux too. One platform's build
  failing no longer stops the others' binaries being attached to the release (#59)

### Fixed

- **Radius and nearest queries returned the wrong rows and distances.** DuckDB 1.x's
  `ST_Distance_Sphere` reads a point as (latitude, longitude), but terrana builds points as
  `ST_Point(longitude, latitude)`, so every radius filter, nearest ranking and `_distance_km`
  (HTTP and the Python `query_radius` / `query_nearest`) used swapped coordinates. On the test
  data a 6.08 km radius returned 14 rows instead of 4. With DuckDB 2.0 and
  `geometry_always_xy = true` they are great-circle distances, and tests now check them against an
  independent haversine. Bounding-box, point-in-polygon and geometry endpoints were not affected
  (#62)
- Radius queries dropped rows at high latitudes: the bounding box that narrows the search used
  the same degree offset for longitude as for latitude, so above about 48° it was narrower than
  the radius east–west (a row 9 km east of 60°N fell outside a 10 km radius). It now widens with
  latitude, and there is no box where the circle reaches a pole or crosses ±180° (#64)
- Windows: files load. `std::fs::canonicalize` returned a verbatim `\\?\C:\...` path, which
  DuckDB reads as a glob and can't open; the CLI now uses `dunce::canonicalize` (#59)
- Windows: the crate builds with MSVC 14.51 (GitHub's current Windows image). DuckDB 1.5.3's
  bundled `fmt` used `stdext::checked_array_iterator`, which that compiler dropped; fixed first
  by DuckDB 1.5.4, and now by the move to DuckDB 2.0 below (#61)
- `--watch` sees changes made right after startup. The watcher was registered on its own thread
  after the server had started answering, so an early change was missed; it is now registered
  before the server binds, and startup fails if it can't be (#63)

## [0.2.12] - 2026-11-10

### Changed

- `cargo test` (and `make ci`) runs the integration tests, which were `#[ignore]`d because the
  server needs DuckDB's spatial extension. It's cached in `~/.duckdb/extensions/` after the first
  download, so they run offline from then on; the test harness installs it once before any
  server starts, and CI caches it. `make test-fast` runs the tests that never need the extension.
  A spatial-extension error now says the first run needs network access (#57)

## [0.2.11] - 2026-11-10

### Added

- Python: `serve()` and `serve_background()` take `token=` and `cors_origins=`, like the CLI's
  `TERRANA_TOKEN`/`--token-file` and `--cors-origin`, and binding a non-loopback address without
  a token raises a `RuntimeWarning` (#50)

### Fixed

- The PyPI page has a project description (a Python-focused README) and links to the
  repository, issues and changelog (#54)
- Python: `serve_background()` raises `OSError` when its port is taken, instead of returning a
  handle to a server that never started and printing the error to stderr (#55)

## [0.2.10] - 2026-11-10

### Added

- An optional bearer token: set `TERRANA_TOKEN` or `--token-file <path>`, and every endpoint but
  `/health` needs `Authorization: Bearer <token>` (401 otherwise). Binding a non-loopback
  address without one logs a warning. `docker-compose.yml` passes `TERRANA_TOKEN` through; an
  empty value counts as no token (#49)
- `--cors-origin <origin>` (repeatable) limits CORS to the listed origins, which may then send
  `Authorization` and read `Link`; `--cors-origin none` turns CORS off. The default is still any
  origin, and now lets browsers send `Authorization` too (#49)

## [0.2.8] - 2026-11-10

### Added

- Richer `where` filters on `GET /query`: `col:ne:`, `gt`, `gte`, `lt`, `lte`,
  `col:between:lo|hi`, `col:in:a|b|c`, `col:nin:a|b|c`, `col:isnull` and `col:notnull`, beside
  the existing `col:value` (also `col:eq:value`). Numbers, booleans, dates and times compare as
  their column's type; a value that isn't valid for its column is a 400 (#45)
- Paging for `GET /query`: `offset=N` skips rows, and when more rows follow the response
  carries a `Link: <?...>; rel="next"` header with the next page's query string. Bodies are unchanged,
  so it works the same for JSON, CSV and GeoJSON; with `nearest=K`, `offset` gives the next K
  nearest (#42)

### Changed

- `where=col:value` on a numeric, boolean, date or time column compares as that type instead
  of as text, so `latitude:36.54` matches `36.5400` and `count:many` is a 400 rather than no
  rows. Text and other columns still compare as text (#45)
- A `where` value that starts with an operator word and a colon (`in:`, `gt:`, ...), or is
  exactly `isnull` or `notnull`, is now read as that operator; write `col:eq:in:progress` to
  match such text. A clause without a colon is a 400 instead of being ignored (#45)
- Library: `terrana::query` takes `where` conditions as `db::filter::Condition`s, built by
  `db::filter::to_conditions`, instead of `(column, value)` pairs (#45)
- `GET /query` results have a fixed order: by distance where there is one, then by `rowid`;
  aggregates by group. Before, bbox, plain and aggregate queries could come back in a
  different order on each request. `nearest`, like `limit`, is capped at 100,000 rows per
  response; the rest are on the next page (#42)
- `GET /query?bbox=...` filters by coordinate ranges instead of the R-tree's `ST_Intersects`,
  which selects the same points. With results sorted, a box covering most of a 1M-row file
  answers in 6 ms instead of 2 s (#42)
- The "listening on" log line reports the address the server bound, so `--port 0` (any free
  port) shows which port it got (#43)

### Fixed

- Repeated `where` parameters all apply; only the last one did (#46)
- An unknown column in `where` is a 400 naming it, instead of a 500 that showed the SQL (#47)
- Integration tests no longer fail now and then with "server did not become ready": two
  parallel tests could be handed the same port. Each test server now binds port 0 and the
  test reads the bound port from its log (#43)

## [0.2.7] - 2026-11-08

### Changed

- Release runs don't save build caches, which only that tag could read and which pushed main's
  out of the cache quota (#40)

## [0.2.6] - 2026-11-08

### Changed

- CI keeps main's build caches warm with two scheduled runs a week, saves caches only from
  `main`, drops the `Check` job that Clippy covers, and cancels a PR's superseded runs (#38)

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

[Unreleased]: https://github.com/jmcmeen/terrana/compare/v0.3.3...HEAD
[0.3.3]: https://github.com/jmcmeen/terrana/compare/v0.3.2...v0.3.3
[0.3.2]: https://github.com/jmcmeen/terrana/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/jmcmeen/terrana/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/jmcmeen/terrana/compare/v0.2.12...v0.3.0
[0.2.12]: https://github.com/jmcmeen/terrana/compare/v0.2.11...v0.2.12
[0.2.11]: https://github.com/jmcmeen/terrana/compare/v0.2.10...v0.2.11
[0.2.10]: https://github.com/jmcmeen/terrana/compare/v0.2.9...v0.2.10
[0.2.8]: https://github.com/jmcmeen/terrana/compare/v0.2.7...v0.2.8
[0.2.7]: https://github.com/jmcmeen/terrana/compare/v0.2.6...v0.2.7
[0.2.6]: https://github.com/jmcmeen/terrana/compare/v0.2.5...v0.2.6
[0.2.1]: https://github.com/jmcmeen/terrana/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/jmcmeen/terrana/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/jmcmeen/terrana/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/jmcmeen/terrana/releases/tag/v0.1.0
