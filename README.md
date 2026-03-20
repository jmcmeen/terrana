# Terrana

Zero-config spatial API server. Point it at a CSV, Parquet, or GeoJSON file containing lat/lon columns and immediately get a REST API with spatial queries and geometry operations — no database setup, no PostGIS, no infrastructure.

```bash
terrana serve observations.csv --lat latitude --lon longitude
# → REST API running at http://localhost:8080
```

## Install

```bash
cargo install --path .
```

## Usage

```bash
terrana serve <FILE> [OPTIONS]
```

### Arguments

| Argument | Description |
|---|---|
| `<FILE>` | Path to CSV, Parquet, GeoJSON, or .duckdb file |

### Options

| Option | Description | Default |
|---|---|---|
| `--lat <COLUMN>` | Latitude column name | auto-detected |
| `--lon <COLUMN>` | Longitude column name | auto-detected |
| `--table <TABLE>` | Table name (DuckDB files only) | — |
| `--port <PORT>` | HTTP port | 8080 |
| `--bind <ADDR>` | Bind address | 127.0.0.1 |
| `--watch` | Re-index when source file changes | off |

### Auto-detection of lat/lon columns

When `--lat` / `--lon` are omitted, column names are scanned case-insensitively:

- **Lat:** `latitude`, `lat`, `y`, `ylat`, `geo_lat`
- **Lon:** `longitude`, `lon`, `lng`, `x`, `xlon`, `xlong`, `geo_lon`, `geo_lng`

## API Endpoints

### Spatial Queries

| Endpoint | Method | Description |
|---|---|---|
| `/query?lat=36.5&lon=-82.5&radius=10km` | GET | Radius search (units: km, m, mi, ft) |
| `/query?bbox=minlat,minlon,maxlat,maxlon` | GET | Bounding box query |
| `/query?lat=36.5&lon=-82.5&nearest=5` | GET | K-nearest neighbors |
| `/query/within` | POST | Point-in-polygon (body: GeoJSON Polygon) |

### Common Query Parameters

| Param | Example | Description |
|---|---|---|
| `select` | `select=species,observed_on` | Column allowlist |
| `where` | `where=quality_grade:research` | Equality filter (repeatable) |
| `group_by` | `group_by=species` | Group column |
| `agg` | `agg=count` or `agg=sum:count` | Aggregation |
| `limit` | `limit=500` | Max rows (default 1000, cap 100000) |
| `format` | `format=json\|csv\|geojson` | Output format |

### Geometry Endpoints

| Endpoint | Description |
|---|---|
| `POST /geometry/area` | Geodesic area + perimeter of polygon(s) |
| `POST /geometry/convex-hull` | Convex hull from points or bbox query |
| `POST /geometry/centroid` | Centroid of any geometry |
| `POST /geometry/buffer` | Geodesic buffer around a point/polygon |
| `POST /geometry/dissolve` | Group-by dissolve → hull per group |
| `POST /geometry/simplify` | Douglas-Peucker / Visvalingam simplification |
| `POST /geometry/distance` | Geodesic distance + bearing between two points |
| `POST /geometry/bounds` | Bounding box, envelope, dimensions |

### Metadata

| Endpoint | Description |
|---|---|
| `GET /health` | Status + uptime |
| `GET /schema` | Column names, types, row count |
| `GET /stats` | Row count, bbox, centroid, index build time |

## Tech Stack

- **Rust** with Axum 0.8 for HTTP
- **DuckDB** (bundled) for file ingestion and SQL queries
- **rstar** R-tree for spatial indexing
- **geo** crate for geodesic geometry (Karney/Vincenty on WGS 84)
- CORS enabled, request tracing via tower-http

## Examples

```bash
# Start server
cargo run -- serve testdata/observations.csv

# Radius search
curl "localhost:8080/query?lat=36.54&lon=-82.54&radius=5km"

# Bounding box as CSV
curl "localhost:8080/query?bbox=35.0,-84.0,37.0,-81.0&format=csv"

# Filtered + projected
curl "localhost:8080/query?bbox=35,-84,37,-81&select=species,observed_on&where=quality_grade:research"

# Geodesic area of a polygon
curl -X POST localhost:8080/geometry/area \
  -H "Content-Type: application/json" \
  -d '{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,1],[0,0]]]}'

# Distance between two points
curl -X POST localhost:8080/geometry/distance \
  -H "Content-Type: application/json" \
  -d '{"from":{"type":"Point","coordinates":[-82.54,36.54]},"to":{"type":"Point","coordinates":[-82.55,36.55]}}'
```

## Docker

```bash
# Drop a CSV into ./data/ and start the server
mkdir -p data
cp your-observations.csv data/observations.csv
docker compose up --build
# → http://localhost:8080
```

The [docker-compose.yml](docker-compose.yml) mounts `./data` into the container and serves whatever file you point it at. Edit the `command` in the compose file to change the filename or add `--lat`/`--lon` overrides.

## Benchmarks

Generate test datasets — simulated iNaturalist-style wildlife observations across the Southern Appalachians:

```bash
# Standard sizes (10K / 100K / 1M)
python3 testdata/generate_benchdata.py

# iNaturalist scale (250M rows, ~15 GB)
python3 testdata/generate_250m.py
```

Run the benchmark suite:

```bash
./bench.sh 1m          # 1M rows on default port 9090
./bench.sh 250m 8080   # 250M rows on port 8080
./bench.sh 100k        # 100K rows, quick smoke test
```

Results on 1M rows (release build, single core):

| Query | Rows | Time |
|---|---|---|
| Index build (1M points) | — | 199ms |
| Nearest 10 | 10 | 11ms |
| Radius 5km | 1,000 | 31ms |
| Radius 10km | 1,000 | 79ms |
| BBox 0.2° | 1,000 | 76ms |
| Within (small polygon) | 20,522 | 119ms |
| Geometry (area/distance/buffer) | 1 | ~5ms |
| Schema / Stats / Health | — | ~5ms |

## License

MIT
