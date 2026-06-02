# Terrana

[![CI](https://github.com/jmcmeen/terrana/actions/workflows/ci.yml/badge.svg)](https://github.com/jmcmeen/terrana/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/terrana.svg)](https://crates.io/crates/terrana)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)


Zero-config spatial API server. Point it at a CSV, Parquet, or GeoJSON file containing lat/lon columns and immediately get a REST API with spatial queries and geometry operations — no database setup, no PostGIS, no infrastructure.

```bash
terrana serve observations.csv --lat latitude --lon longitude
# → REST API running at http://localhost:8080
```

## Installing Rust

Terrana is built with Rust. If you don't already have a toolchain, install one with
[rustup](https://rustup.rs) — the official Rust installer:

```bash
# macOS / Linux
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

On **Windows**, download and run [`rustup-init.exe`](https://rustup.rs) (or
`winget install Rustlang.Rustup`). Then restart your shell and confirm it worked:

```bash
rustc --version    # e.g. rustc 1.82.0
cargo --version
```

`rustup` installs `cargo` (the build tool / package manager) alongside the compiler.
That's all you need — Terrana bundles DuckDB, so there's **no system DuckDB, PostGIS,
or other infrastructure to install**. (On first run, DuckDB downloads its `spatial`
extension from the network and caches it locally.)

## Install

Terrana is published on [crates.io](https://crates.io/crates/terrana). Install the
`terrana` binary with:

```bash
cargo install terrana
```

Or build and install from a checkout of this repository:

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
| `--disk` | Use on-disk DuckDB storage (reduces RAM for large files) | off |

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
- **DuckDB** (bundled) for file ingestion, SQL queries, and spatial R-tree indexing
- **DuckDB spatial extension** for R-tree index, `ST_Intersects`, `ST_Distance_Sphere`, `ST_Contains`
- **geo** crate for geodesic geometry (area, buffer, distance endpoints)
- CORS enabled, request tracing via tower-http

## Examples

```bash
# Start server
cargo run -- serve testdata/observations.csv

# Large files — use --disk to keep DuckDB on disk and reduce RAM usage
cargo run -- serve huge_dataset.csv --disk

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

## Citation

If you use Terrana in academic research, please cite it as:

```bibtex
@software{terrana,
  title  = {Terrana: Zero-Config Spatial API Server},
  url    = {https://github.com/jmcmeen/terrana},
  year   = {2026},
  note   = {Rust-based spatial query server using DuckDB and R-tree indexing}
}
```

Or in prose: *Terrana (2026). Zero-config spatial API server. <https://github.com/jmcmeen/terrana>*

## Testing

```bash
cargo test                       # fast unit tests (offline)
cargo test -- --include-ignored  # + integration tests (start the server; need network)
```

The integration tests in [tests/api.rs](tests/api.rs) spawn the real binary and hit
the HTTP endpoints. They are `#[ignore]`d by default because starting the server
downloads the DuckDB `spatial` extension on first use.

## Contributing

Contributions are welcome — bug reports, new file formats, geometry operations,
performance work, and docs. See [CONTRIBUTING.md](CONTRIBUTING.md) for development
setup and the pre-PR checklist, and [SECURITY.md](SECURITY.md) for how to report
vulnerabilities.

```bash
git clone https://github.com/jmcmeen/terrana.git
cd terrana
cargo build
cargo run -- serve testdata/observations.csv
```

Please run `cargo fmt --all` and `cargo clippy --all-targets -- -D warnings` before
submitting a PR.

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) at your
option. Unless you explicitly state otherwise, any contribution you submit for
inclusion in this work shall be dual-licensed as above, without additional terms.
