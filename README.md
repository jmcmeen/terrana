# Terrana

[![CI](https://github.com/jmcmeen/terrana/actions/workflows/ci.yml/badge.svg)](https://github.com/jmcmeen/terrana/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/terrana.svg?color=fc8d62)](https://crates.io/crates/terrana)
[![docs.rs](https://img.shields.io/docsrs/terrana?color=e78ac3)](https://docs.rs/terrana)
[![PyPI](https://img.shields.io/pypi/v/terrana.svg?color=4c8cb5)](https://pypi.org/project/terrana/)
[![Python versions](https://img.shields.io/badge/python-3.9%20%7C%203.10%20%7C%203.11%20%7C%203.12%20%7C%203.13-66c2a5.svg)](https://pypi.org/project/terrana/)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-8da0cb.svg)](#license)
[![DOI](https://zenodo.org/badge/DOI/10.5281/zenodo.20515989.svg)](https://doi.org/10.5281/zenodo.20515989)

Zero-config spatial API server. Point it at a CSV, Parquet, or GeoJSON file containing lat/lon columns and immediately get a REST API with spatial queries and geometry operations — no database setup, no PostGIS, no infrastructure.

```bash
terrana serve observations.csv --lat latitude --lon longitude
# → REST API running at http://localhost:8080
```

Terrana is three things in one: a **CLI / HTTP server** (above), a **Rust library**
(`cargo add terrana`), and a **Python package** (`pip install terrana`) — the same
engine, embeddable in-process or served over HTTP.

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

### Prebuilt binaries

Each [release](https://github.com/jmcmeen/terrana/releases/latest) has a binary for:

| Platform | File |
|---|---|
| Linux x86_64 (glibc 2.28+) | `terrana-x86_64-unknown-linux-gnu.tar.gz` |
| Linux arm64 (glibc 2.28+) | `terrana-aarch64-unknown-linux-gnu.tar.gz` |
| macOS Apple Silicon (11+) | `terrana-aarch64-apple-darwin.tar.gz` |
| macOS Intel (10.13+) | `terrana-x86_64-apple-darwin.tar.gz` |
| Windows x86_64 | `terrana-x86_64-pc-windows-msvc.zip` |

The Linux binaries need glibc 2.28 or newer: RHEL 8+, Debian 10+, Ubuntu 18.10+, Amazon
Linux 2023. `pip install terrana` installs the same `terrana` command on all of these (see
[Python](#python)).

### As a library dependency

Terrana is a `lib + bin` crate — add it to your own Rust project to use its
ingestion, spatial queries, and geodesic geometry directly (see
[Rust library](#rust-library) below for usage):

```bash
cargo add terrana                        # includes the embedded Axum server
cargo add terrana --no-default-features  # pure library, without axum / tokio
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
| `--cors-origin <ORIGIN>` | Browser origin allowed to call the API (repeatable); `none` turns CORS off | any origin |
| `--token-file <PATH>` | Require a bearer token, read from the file's first line (or set `TERRANA_TOKEN`) | no token |

### Access control

On the default `127.0.0.1` only this machine can reach the server. Before binding anything
else (`--bind 0.0.0.0`, Docker), set a token. Every endpoint except `/health` then needs it:

```bash
export TERRANA_TOKEN="$(openssl rand -hex 32)"
terrana serve observations.csv --bind 0.0.0.0
curl -H "Authorization: Bearer $TERRANA_TOKEN" "localhost:8080/query?limit=5"
```

Use `TERRANA_TOKEN` or `--token-file`; there's no `--token` flag, because a token on the
command line shows in the process list. A missing or wrong token gets a 401. `/health` stays
open for load balancers and container health checks. Binding a non-loopback address without a
token logs a warning.

CORS allows any origin by default. To let only your own web app call the API from a browser,
list its origins: `--cors-origin https://maps.example.org --cors-origin http://localhost:3000`.
With a list, browsers may send the `Authorization` header and read `Link` (for paging).
`--cors-origin none` sends no CORS headers at all.

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
| `where` | `where=count:gte:5` | Filters: equality, comparisons, ranges, lists, null tests (repeatable; see below) |
| `group_by` | `group_by=species` | Group column |
| `agg` | `agg=count` or `agg=sum:count` | Aggregation |
| `limit` | `limit=500` | Max rows (default 1000, cap 100000) |
| `offset` | `offset=1000` | Rows to skip, for paging (see below) |
| `format` | `format=json\|csv\|geojson` | Output format |

### Filtering

`where` takes comma-separated clauses, and you can repeat it; every clause must match.
Numbers, booleans, dates and times compare as their column's type (see `/schema`); other
columns compare as text. Column names match whatever their case:

| Clause | Matches |
|---|---|
| `col:value` or `col:eq:value` | equal |
| `col:ne:value` | not equal |
| `col:gt:value`, `gte`, `lt`, `lte` | greater, at least, less, at most |
| `col:between:lo\|hi` | `lo` to `hi`, inclusive |
| `col:in:a\|b\|c`, `col:nin:a\|b\|c` | in the list, not in it |
| `col:isnull`, `col:notnull` | empty, not empty |

```
curl "localhost:8080/query?where=count:gte:5,observed_on:between:2024-03-15|2024-03-20"
curl "localhost:8080/query?where=species:in:Barred%20Owl|Black%20Bear&where=quality_grade:research"
```

A value that isn't valid for its column (`count:gt:many`) or an unknown column is a 400. As in
SQL, `ne` and `nin` don't match empty values. To match a value that looks like an operator,
use `eq`: `note:eq:gt:5`. Values can't contain a comma, nor `|` inside a list.

### Paging

Every `/query` result has a fixed order: nearest first for radius and nearest queries, then
by row (the `rowid` column, numbered as the file is loaded); aggregates by group. `offset` skips
that many rows. When more rows follow, the response has a `Link` header (RFC 8288) pointing
at the next page, with your other parameters kept. The target is relative (`?...`), so it
resolves against whatever path you called, including behind a proxy:

```
$ curl -i "localhost:8080/query?bbox=35,-84,37,-81&limit=100"
link: <?bbox=35,-84,37,-81&limit=100&offset=100>; rel="next"
```

The body is unchanged (JSON array, CSV or GeoJSON), so the header works the same for every
format; no `Link` header means the last page. With `nearest=K`, `offset` gives the next K
nearest; `nearest`, like `limit`, is capped at 100,000 per page. Offsets count rows in the data loaded now: if `--watch` reloads the file between
pages, rows can shift.

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

## Rust library

The same engine is available as a library — load a file, then run spatial queries
and geodesic geometry without the HTTP server:

```rust
use terrana::{db, ingest_file};
use terrana::geometry::measure::geodesic_distance;
use geo_types::Point; // Terrana's geometry operates on geo-types

// Load a CSV / Parquet / GeoJSON file and build the spatial R-tree index.
let conn = db::create_connection()?;
let info = ingest_file(&conn, "observations.csv".as_ref(), None, None, None)?;
println!("loaded {} rows (lat={}, lon={})", info.row_count, info.lat_col, info.lon_col);

// Geodesic distance + bearing (WGS 84, never planar).
let d = geodesic_distance(Point::new(-82.54, 36.54), Point::new(-82.55, 36.55));
println!("{:.0} m at bearing {:.1}°", d.distance_m, d.bearing_deg);
```

Spatial queries run through the DuckDB-backed builders in `terrana::db::query`
(`bbox_filter` / `radius_filter` + `query`); geodesic geometry lives in
`terrana::geometry` (`area`, `buffer`, `hull`, `simplify`, `measure`). The `server`
feature (the Axum router, on by default) can be disabled for a pure-library
dependency — `cargo add terrana --no-default-features`.

## Python

Terrana ships Python bindings backed by the same Rust engine — no separate Rust toolchain needed:

```bash
pip install terrana        # or: uv pip install terrana
```

The wheel is a single stable-ABI (`abi3`) build that runs on CPython 3.9 – 3.14+.

### Command line

The wheel installs a `terrana` command — the same CLI as the Rust binary, with no Rust
toolchain required:

```bash
pip install terrana
terrana serve observations.csv --lat latitude --lon longitude
# → REST API running at http://localhost:8080
```

Every option the binary accepts works here too (`--port`, `--bind`, `--watch`, `--disk`,
`--table`, `--lat`, `--lon`) — see [Usage](#usage) above. `Ctrl-C` stops the server.

### Library mode (in-process, no server)

```python
import terrana, json

session = terrana.load_csv("observations.csv")   # also load_parquet / load_geojson
print(session.row_count, session.lat_col, session.lon_col)

# Spatial queries take (lat, lon) — mirroring the REST API's ?lat=&lon=.
# Each row is a dict; radius/nearest add a `_distance_km` key.
rows = session.query_radius(36.54, -82.54, 5000)          # lat, lon, radius_m
box  = session.query_bbox(35.0, -84.0, 37.0, -81.0)       # min_lat, min_lon, max_lat, max_lon
near = session.query_nearest(36.5, -82.5, 5)              # lat, lon, k

# Geodesic geometry takes (lon, lat) — GeoJSON coordinate order. GeoJSON outputs
# come back as JSON strings (use json.loads); area/distance come back as dicts.
d    = terrana.geodesic_distance(-82.54, 36.54, -82.55, 36.55)   # lon1, lat1, lon2, lat2
a    = terrana.geodesic_area('{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,1],[0,0]]]}')
hull = json.loads(terrana.convex_hull('{"type":"MultiPoint","coordinates":[[0,0],[1,0],[0,1]]}'))
buf  = json.loads(terrana.buffer(-82.5, 36.5, 5000, segments=64))  # lon, lat, distance_m
```

> **Coordinate order:** spatial queries are `(lat, lon)` (REST-API order); geometry
> functions are `(lon, lat)` (GeoJSON order). Mind the difference.

### Server mode (embedded HTTP server)

Run the full REST API in a background thread and manage it from Python:

```python
import terrana, requests

session = terrana.load_csv("observations.csv")

with session.serve_background(port=8080) as server:
    # The complete REST API is live at http://localhost:8080
    r = requests.get("http://localhost:8080/query?lat=36.5&lon=-82.5&radius=10km")
    print(r.json())
# server is shut down and its thread joined on context exit

# Or manage the lifecycle explicitly:
server = session.serve_background(port=8080)
# ... use the API ...
server.shutdown()       # stops the server and joins the thread (idempotent)

# Or block in the foreground until Ctrl-C:
# session.serve(port=8080)
```

The tokio runtime lives entirely inside a Rust thread, so serving never blocks Python's GIL.

`serve` and `serve_background` take the same access controls as the command line (see
[Access control](#access-control)):

```python
session.serve_background(
    port=8080,
    bind="0.0.0.0",
    token=os.environ.get("TERRANA_TOKEN") or None,  # every endpoint but /health needs it
    cors_origins=["https://maps.example.org"],    # default: any origin; ["none"] turns CORS off
)
```

An empty token, an empty `cors_origins` list or a malformed origin raises `ValueError`, and a
port that's already taken raises `OSError`. Binding a non-loopback address without a token
raises a `RuntimeWarning`. The embedded server doesn't read `TERRANA_TOKEN`
itself; pass the token explicitly.

## Docker

```bash
# Drop a CSV into ./data/ and start the server
mkdir -p data
cp your-observations.csv data/observations.csv
docker compose up --build
# → http://localhost:8080
```

The [docker-compose.yml](docker-compose.yml) mounts `./data` into the container and serves whatever file you point it at. Edit the `command` in the compose file to change the filename or add `--lat`/`--lon` overrides. The container binds `0.0.0.0`, so set `TERRANA_TOKEN` (see [Access control](#access-control)); the compose file passes it through from your shell or a `.env` file.

## Benchmarks

Generate test datasets — simulated iNaturalist-style wildlife observations across the Southern Appalachians:

```bash
# Standard sizes (10K / 100K / 1M)
python3 testdata/generate.py --preset bench

# iNaturalist scale (250M rows, ~15 GB)
python3 testdata/generate.py --preset 250m
```

Run the benchmark suite:

```bash
./testdata/bench.sh 1m          # 1M rows on default port 9090
./testdata/bench.sh 250m 8080   # 250M rows on port 8080
./testdata/bench.sh 100k        # 100K rows, quick smoke test
```

Or via `make`: `make gen` to build the datasets and `make bench DATASET=1m` to run the suite.

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
  title     = {Terrana: Zero-Config Spatial API Server},
  url       = {https://github.com/jmcmeen/terrana},
  doi       = {10.5281/zenodo.20515989},
  year      = {2026},
  publisher = {Zenodo},
  note      = {Rust-based spatial query server using DuckDB and R-tree indexing}
}
```

Or in prose: *Terrana (2026). Zero-config spatial API server. <https://doi.org/10.5281/zenodo.20515989>*

## Testing

```bash
cargo test     # unit + integration tests
```

The integration tests in [tests/api.rs](tests/api.rs) spawn the real binary and hit
the HTTP endpoints. Starting the server needs DuckDB's `spatial` extension, which DuckDB
downloads on first use and caches in `~/.duckdb/extensions/`. So the first run needs network
access; after that the tests, like the server, run offline.

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
