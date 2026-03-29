# Changelog

All notable changes to Terrana will be documented in this file.

## [Unreleased]

### Changed

- **Replaced in-memory rstar R-tree with DuckDB spatial R-tree index** — spatial indexing is now handled entirely by DuckDB's spatial extension (`CREATE INDEX ... USING RTREE`). This eliminates the ~18GB RAM overhead at 250M rows and removes the startup cost of scanning all rows into a Rust-side index.
- Query path distances (radius, nearest) now use haversine (`ST_Distance_Sphere`) instead of Vincenty. Accurate to ~0.3% — sufficient for spatial filtering. Geometry endpoints still use ellipsoidal math.
- All spatial queries are now single SQL statements with spatial predicates (`ST_Intersects`, `ST_Contains`, `ST_Distance_Sphere`) instead of the previous two-stage R-tree prune → DuckDB fetch pattern.

### Added

- `--disk` flag for `terrana serve` — uses on-disk DuckDB storage instead of in-memory, reducing RAM usage for large datasets. DuckDB spatial index is also stored on disk.

### Removed

- `rstar` and `rayon` dependencies — no longer needed with DuckDB-managed spatial index.
- `src/index/` module (`SpatialPoint`, `build_rtree`) — replaced by `db::loader::add_spatial_index()`.

## [0.1.0] - 2026-03-20

### Added

- CLI with `terrana serve` subcommand and `--lat`, `--lon`, `--port`, `--bind`, `--watch` options
- Auto-detection of lat/lon columns from common naming conventions
- In-memory file ingestion for CSV and GeoJSON
- R-tree spatial index (rstar) with bulk loading
- `GET /query` endpoint with radius, bounding box, and nearest neighbor modes
- `POST /query/within` for point-in-polygon queries
- `select=`, `where=`, `group_by=`, `agg=`, `limit=` query parameters
- JSON, CSV, and GeoJSON output formats
- Geometry endpoints: area, convex-hull, centroid, buffer, dissolve, simplify, distance, bounds
- All geometry calculations use geodesic algorithms (WGS 84 ellipsoid)
- `GET /health`, `GET /schema`, `GET /stats` metadata endpoints
- CORS support and request tracing via tower-http
- Tracing/logging with `RUST_LOG` env filter
- GitHub Actions CI (check, clippy, fmt, cross-platform build) and release workflows
- Dockerfile and docker-compose.yml for containerized deployment


### Security

- Column name validation for all user-supplied column names, group_by, agg, and select params

### Fixed

- Empty R-tree no longer panics in `/stats` endpoint
- Invalid bbox coordinate values now return 400 instead of silently defaulting to 0.0
- NaN-safe sorting in radius and nearest-neighbor queries (no more panic on degenerate coordinates)

### Improved

- `/query/within` uses R-tree envelope pre-filtering before precise point-in-polygon tests
