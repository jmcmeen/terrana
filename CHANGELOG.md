# Changelog

All notable changes to Terrana will be documented in this file.

## [0.1.0] - 2026-03-20

### Added

- CLI with `terrana serve` subcommand and `--lat`, `--lon`, `--table`, `--port`, `--bind`, `--watch` options
- Auto-detection of lat/lon columns from common naming conventions
- DuckDB-based file ingestion for CSV, Parquet, GeoJSON, and .duckdb files
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
