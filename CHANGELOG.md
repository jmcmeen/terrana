# Changelog

All notable changes to Terrana will be documented in this file.

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
