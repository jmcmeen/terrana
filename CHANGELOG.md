# Changelog

All notable changes to Terrana will be documented in this file.

## [0.1.0] - 2026-03-28

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

### Security

- Column name validation for all user-supplied column names, group_by, agg, and select params

### Known Issues

- DuckDB spatial extension (`ST_Point`) crashes on large in-memory tables (250M+ rows) due to a `StringStats` assertion bug in duckdb crate v1.10501.0. Workaround: use `--disk` flag, which stores data on disk and avoids the in-memory crash. See BACKLOG.md for details.
