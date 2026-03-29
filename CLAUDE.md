# CLAUDE.md — Terrana Project Context

## IMPORTANT: Project Boundaries

**This project is `terrana/`. Do not read, modify, or reference files outside of this directory.** If you notice other folders in the workspace, ignore them entirely. All work happens here.

If you need a file that doesn't exist yet, create it. If you're unsure where something belongs, refer to the project structure section below.

---

## What This Project Is

**Terrana** is a zero-config spatial API server written in Rust. You point it at a CSV, Parquet, or GeoJSON file containing lat/lon columns and immediately get a REST API with spatial queries and geometry operations — no database setup, no PostGIS, no infrastructure.

```bash
terrana serve observations.csv --lat latitude --lon longitude
# → REST API running at http://localhost:8080
```

The binary is called `terrana`. The crate name is `terrana`.

---

## Tech Stack

| Concern | Choice |
|---|---|
| Language | Rust (edition 2021) |
| HTTP framework | `axum` 0.7 |
| Async runtime | `tokio` (full features) |
| Database engine | `duckdb` crate (bundled feature — no system DuckDB needed) |
| Spatial index | `rstar` (R-tree, pure Rust) |
| Geometry / geodesics | `geo` crate |
| GeoJSON types | `geojson` crate (with `geo-types` feature) |
| CLI | `clap` 4 (derive API) |
| Serialization | `serde` + `serde_json` |
| CSV output | `csv` crate |
| Error handling | `anyhow` (app-level) + `thiserror` (typed errors) |
| Logging | `tracing` + `tracing-subscriber` |
| Parallel index build | `rayon` |
| File watching | `notify` 6 |
| Coordinate system | WGS 84 (EPSG:4326) only — no CRS conversion |

---

## Cargo.toml (authoritative)

```toml
[package]
name = "terrana"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "terrana"
path = "src/main.rs"

[dependencies]
clap = { version = "4", features = ["derive"] }
tokio = { version = "1", features = ["full"] }
axum = "0.8"
tower = "0.5"
tower-http = { version = "0.6", features = ["cors", "trace"] }
duckdb = { version = "1", features = ["bundled"] }
rstar = "0.12"
geo = "0.29"
geo-types = "0.7"
geojson = { version = "1", features = ["geo-types"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
csv = "1"
chrono = "0.4"
anyhow = "1"
thiserror = "2"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
rayon = "1"
notify = "8"
```

---

## Project Structure

Every file that should exist is listed here. Do not create files outside this tree.

```
terrana/
├── CLAUDE.md                  ← this file
├── Cargo.toml
├── Cargo.lock
├── README.md
├── testdata/
│   ├── observations.csv       ← lat/lon point data (id, species, observed_on, quality_grade, latitude, longitude, count)
│   └── parks.geojson          ← polygon features for testing /query/within and /geometry/area
└── src/
    ├── main.rs                ← entry point: parse CLI args, build config, start server
    ├── cli.rs                 ← clap arg structs and subcommands
    ├── config.rs              ← resolved Config struct passed through app via Arc
    ├── error.rs               ← AppError enum implementing IntoResponse
    ├── db/
    │   ├── mod.rs             ← DuckDB connection setup, re-exports
    │   ├── loader.rs          ← file ingestion by extension (CSV/Parquet/GeoJSON/DuckDB)
    │   └── query.rs           ← parameterized SQL query builders
    ├── index/
    │   ├── mod.rs             ← SpatialPoint type, RTree re-export
    │   └── build.rs           ← bulk R-tree construction from DuckDB scan
    ├── geometry/
    │   ├── mod.rs             ← re-exports, shared helpers
    │   ├── hull.rs            ← convex hull computation
    │   ├── area.rs            ← geodesic area and perimeter
    │   ├── buffer.rs          ← geodesic buffer via GeodesicDestination
    │   ├── dissolve.rs        ← group-by dissolve → hull per group
    │   ├── simplify.rs        ← Douglas-Peucker simplification
    │   └── measure.rs         ← geodesic distance, bearing, centroid, bounds
    ├── server/
    │   ├── mod.rs             ← axum router assembly, AppState definition
    │   └── middleware.rs      ← CORS, request logging
    ├── handlers/
    │   ├── query.rs           ← GET /query (radius, bbox, nearest)
    │   ├── within.rs          ← POST /query/within
    │   ├── geometry.rs        ← POST /geometry/* dispatch
    │   └── meta.rs            ← GET /health, GET /schema, GET /stats
    └── output/
        ├── mod.rs             ← OutputFormat enum, dispatch
        ├── json.rs            ← JSON rows response
        ├── csv.rs             ← CSV response using csv crate
        └── geojson.rs         ← GeoJSON FeatureCollection response
```

---

## AppState

The axum router shares this state across all handlers. Clone it freely — the expensive parts are behind `Arc`.

```rust
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: Arc<Mutex<duckdb::Connection>>,  // DuckDB Connection is not Send
    pub index: Arc<RTree<SpatialPoint>>,
    pub schema: Arc<TableSchema>,
}

pub struct TableSchema {
    pub source: String,
    pub row_count: i64,
    pub lat_col: String,
    pub lon_col: String,
    pub columns: Vec<ColumnMeta>,
}

pub struct ColumnMeta {
    pub name: String,
    pub dtype: String,  // DuckDB type string e.g. "VARCHAR", "DOUBLE", "DATE"
}
```

---

## Error Handling

All errors flow through `AppError` in `src/error.rs`. All handlers return `Result<impl IntoResponse, AppError>`.

```rust
#[derive(thiserror::Error, Debug)]
pub enum AppError {
    #[error("Invalid query parameter: {0}")]
    BadRequest(String),
    #[error("Column not found: {0}")]
    ColumnNotFound(String),
    #[error("File not found: {0}")]
    FileNotFound(String),
    #[error("Geometry error: {0}")]
    Geometry(String),
    #[error("Database error: {0}")]
    Database(#[from] duckdb::Error),
    #[error("Internal error: {0}")]
    Internal(#[from] anyhow::Error),
}

// HTTP status mapping:
// BadRequest | ColumnNotFound → 400
// FileNotFound               → 404
// everything else            → 500
```

---

## Spatial Index

```rust
#[derive(Clone, Debug)]
pub struct SpatialPoint {
    pub rowid: i64,
    pub lat: f64,
    pub lon: f64,
}

impl RTreeObject for SpatialPoint {
    type Envelope = AABB<[f64; 2]>;
    fn envelope(&self) -> Self::Envelope {
        AABB::from_point([self.lon, self.lat])  // note: lon first (x), lat second (y)
    }
}
```

Build with `RTree::bulk_load()` after collecting all points via rayon. Log build time at INFO level.

---

## Query Flow (Spatial Endpoints)

All spatial queries follow this two-stage pattern. Do not deviate from it:

1. Parse query params into a typed struct
2. Use R-tree to get a candidate `Vec<i64>` of rowids within the search envelope
3. Run `SELECT * FROM data WHERE rowid IN (?)` in DuckDB to fetch full rows for candidates only
4. If radius query: filter candidates by precise geodesic distance using `geo::GeodesicDistance`; inject `_distance_km` field
5. Apply `where=`, `select=`, `limit=`
6. Serialize to requested output format

Never full-scan DuckDB for spatial queries. Always prune with the R-tree first.

---

## CLI Interface

```
terrana serve <FILE> [OPTIONS]

Arguments:
  <FILE>   Path to CSV, Parquet, GeoJSON, or .duckdb file

Options:
  --lat <COLUMN>      Latitude column name [auto-detected if omitted]
  --lon <COLUMN>      Longitude column name [auto-detected if omitted]
  --table <TABLE>     Table name (DuckDB files only)
  --port <PORT>       HTTP port [default: 8080]
  --bind <ADDR>       Bind address [default: 127.0.0.1]
  --watch             Re-index when source file changes
  --disk              Use on-disk DuckDB storage (reduces RAM for large files)
  -h, --help          Print help
  -V, --version       Print version
```

### Auto-detection of lat/lon columns

When `--lat` / `--lon` are omitted, scan column names case-insensitively in this priority order:

- **Lat:** `latitude`, `lat`, `y`, `ylat`, `geo_lat`
- **Lon:** `longitude`, `lon`, `lng`, `x`, `xlon`, `xlong`, `geo_lon`, `geo_lng`

If detection fails, print a clear error listing all available column names and exit non-zero.

---

## REST API Reference

### Spatial Query Endpoints

#### `GET /query` — Radius search
```
?lat=36.5&lon=-82.5&radius=10km
```
Units: `km`, `m`, `mi`, `ft`. Adds `_distance_km` to each row. Sorted ascending by distance.

#### `GET /query` — Bounding box
```
?bbox=minlat,minlon,maxlat,maxlon
```

#### `GET /query` — Nearest neighbor
```
?lat=36.5&lon=-82.5&nearest=5
```
Returns N nearest rows. Adds `_distance_km`. Sorted ascending.

#### `POST /query/within` — Point-in-polygon
Body: GeoJSON `Polygon`, `MultiPolygon`, `Feature`, or `FeatureCollection`. Uses `geo::Contains` for the pip test.

### Common Query Parameters

| Param | Description | Example |
|---|---|---|
| `select` | Column allowlist (comma-separated) | `select=species,observed_on` |
| `where` | Equality filter (repeatable) | `where=quality_grade:research` |
| `group_by` | Group column | `group_by=species` |
| `agg` | Aggregation | `agg=count` or `agg=sum:count` |
| `limit` | Max rows (default 1000, hard cap 100000) | `limit=500` |
| `format` | Output format | `format=json` \| `format=csv` \| `format=geojson` |

### Geometry Endpoints

All geometry operations use geodesic algorithms from the `geo` crate. Never use planar/Cartesian math for area, distance, or buffer calculations.

#### `POST /geometry/convex-hull`
Input: GeoJSON FeatureCollection or `{ "query": { "bbox": [...] } }`.
Output: hull polygon Feature with `area_m2`, `area_km2`, `area_ha`, `perimeter_m`, `point_count` in properties.

#### `POST /geometry/area`
Input: GeoJSON Polygon, MultiPolygon, Feature, or FeatureCollection.
Output: `{ area_m2, area_km2, area_ha, area_acres, perimeter_m }`.
Uses `geo::GeodesicArea::geodesic_area_unsigned()` — Karney's algorithm, WGS 84 ellipsoid.

#### `POST /geometry/centroid`
Input: any GeoJSON geometry.
Output: `{ "centroid": { "type": "Point", "coordinates": [lon, lat] } }`.

#### `POST /geometry/buffer`
Input: `{ "geometry": <GeoJSON Point or Polygon>, "distance": 5000, "unit": "m", "segments": 64 }`.
Geodesic buffer: shoot rays via `geo::GeodesicDestination` at each bearing, close the ring.
Units: `m` (default), `km`, `mi`, `ft`.

#### `POST /geometry/dissolve`
Input: `{ "query": { "bbox": [...] }, "by": "species", "include_area": true, "include_count": true }`.
Groups rows by attribute, computes convex hull per group, returns FeatureCollection.

#### `POST /geometry/simplify`
Input: `{ "geometry": <GeoJSON Polygon>, "tolerance": 0.001, "preserve_topology": true }`.
Tolerance in degrees. Use `geo::Simplify` or `geo::SimplifyVw` based on `preserve_topology`.

#### `POST /geometry/distance`
Input: `{ "from": <GeoJSON Point>, "to": <GeoJSON Point> }`.
Output: `{ distance_m, distance_km, distance_mi, bearing_deg }`.
Uses `geo::GeodesicDistance` (Vincenty) and `geo::Bearing`.

#### `POST /geometry/bounds`
Input: any GeoJSON geometry.
Output: `{ "bbox": [minlat, minlon, maxlat, maxlon], "envelope": <GeoJSON Polygon>, "width_km", "height_km", "area_km2" }`.

### Metadata Endpoints

| Endpoint | Description |
|---|---|
| `GET /health` | `{ "status": "ok", "uptime_s": 142 }` |
| `GET /schema` | Column names, types, lat/lon column names, row count |
| `GET /stats` | Row count, bbox, centroid, index build time |

---

## Geodesic Rules (Non-Negotiable)

These apply to every geometry calculation in this codebase:

- **Area** → `geo::GeodesicArea::geodesic_area_unsigned()`. Never planar.
- **Distance** → `geo::GeodesicDistance::geodesic_distance()`. Never Euclidean.
- **Buffer ring vertices** → `geo::GeodesicDestination::geodesic_destination(origin, bearing, distance_m)`.
- **Convex hull shape** → computed on 2D lat/lon (acceptable); area of that hull → geodesic.
- **Haversine is acceptable only** for R-tree envelope expansion. All distance values reported to users must use Vincenty via `geo::GeodesicDistance`.

---

## DuckDB Ingestion by File Type

```rust
match extension {
    "csv"     => conn.execute("CREATE VIEW data AS SELECT * FROM read_csv_auto(?)", [path])?,
    "parquet" => conn.execute("CREATE VIEW data AS SELECT * FROM read_parquet(?)", [path])?,
    "geojson" | "json" => {
        conn.execute("LOAD spatial", [])?;
        conn.execute("CREATE VIEW data AS SELECT * FROM ST_Read(?)", [path])?;
    }
    "duckdb"  => {
        // Open the file directly, use --table arg to select the table
        // Wrap in a view called `data` for uniform downstream SQL
    }
    _ => return Err(AppError::BadRequest(format!("Unsupported file type: {}", extension)))
}
```

All downstream SQL always queries the `data` view. This keeps the query builder uniform regardless of source.

---

## Output Format Behavior

| Format | Content-Type | Notes |
|---|---|---|
| `json` | `application/json` | Array of row objects. Default. |
| `csv` | `text/csv` | Header row + data rows via `csv` crate writer. |
| `geojson` | `application/geo+json` | FeatureCollection. Each row is a Feature with Point geometry at its lat/lon. All other columns go into `properties`. |

---

## Build Phases (implement in order, verify each before proceeding)

### Phase 1 — CLI Skeleton
`clap` args, `Config` struct, `tracing` setup, startup banner.
**Gate:** `cargo run -- serve --help` prints usage correctly.

### Phase 2 — DuckDB Integration
File ingestion, `GET /schema`, `GET /stats`, column auto-detection.
**Gate:** `curl localhost:8080/schema` returns column list for a test CSV.

### Phase 3 — R-tree Index
Bulk load from DuckDB scan via rayon, stored in AppState.
**Gate:** index builds in under 1 second on a 100k-row CSV; build time logged.

### Phase 4 — Core Spatial Queries
`GET /query` (radius, bbox, nearest), `POST /query/within`.
**Gate:** radius query returns correct rows verified against known test data; `_distance_km` is present and sorted.

### Phase 5 — Output Formats + Filters
CSV, GeoJSON output; `select=`, `where=`, `limit=`, `group_by=`, `agg=`.
**Gate:** `?format=geojson` response validates as GeoJSON; `?format=csv` is well-formed.

### Phase 6 — Geometry Endpoints
All `POST /geometry/*` routes.
**Gate:** `/geometry/area` on a 1°×1° box at the equator returns ~12,308 km².

### Phase 7 — Polish
`GET /health`, `--watch` mode, CORS, request logging middleware.
**Gate:** `cargo build --release` produces a single binary with no external dependencies.

---

## What This Project Is NOT

Do not implement any of the following. If a request seems to push toward them, flag it and stop.

- No replacement for PostGIS (no coordinate transforms, no topology operations beyond what `geo` provides)
- No distributed system (single node, single file, single process)
- No tile server (no MVT/raster output)
- No write support (read-only; no append, no insert)
- No CRS conversion (WGS 84 only, no `proj` dependency)
- No H3 hexagonal indexing (R-tree only for MVP)
- No Python/R bindings (no `pyo3`)
- No map rendering

---

## Running the Project

```bash
# Development
cargo run -- serve testdata/observations.csv

# With explicit columns and custom port
cargo run -- serve testdata/observations.csv --lat latitude --lon longitude --port 9000

# Large files — use --disk to keep DuckDB on disk and reduce RAM
cargo run -- serve huge_dataset.csv --disk

# Release build
cargo build --release
./target/release/terrana serve testdata/observations.csv

# Log levels
RUST_LOG=debug cargo run -- serve testdata/observations.csv
RUST_LOG=terrana=info cargo run -- serve testdata/observations.csv
```

---

## Acceptance Tests

```bash
# Phase 1
cargo run -- serve --help

# Phase 2
curl localhost:8080/schema
curl localhost:8080/stats

# Phase 4
curl "localhost:8080/query?lat=36.5&lon=-82.5&radius=10km"
curl "localhost:8080/query?bbox=35.0,-84.0,37.0,-81.0"
curl "localhost:8080/query?lat=36.5&lon=-82.5&nearest=5"

# Phase 5
curl "localhost:8080/query?bbox=35.0,-84.0,37.0,-81.0&format=geojson"
curl "localhost:8080/query?bbox=35.0,-84.0,37.0,-81.0&format=csv"
curl "localhost:8080/query?bbox=35.0,-84.0,37.0,-81.0&select=species,observed_on&where=quality_grade:research"

# Phase 6 — geodesic area sanity check
# A 1° x 1° box at the equator should be ~12,308 km²
curl -X POST localhost:8080/geometry/area \
  -H "Content-Type: application/json" \
  -d '{"geometry":{"type":"Polygon","coordinates":[[[-1,0],[1,0],[1,1],[-1,1],[-1,0]]]}}'
```
