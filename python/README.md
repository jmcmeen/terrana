# terrana

Spatial queries and geodesic geometry for Python, backed by a Rust engine (DuckDB with its
spatial extension, and the `geo` crate). Point it at a CSV, Parquet or GeoJSON file with
lat/lon columns and query it in-process, or serve it as a REST API. No database to set up, no
PostGIS.

```bash
pip install terrana        # or: uv pip install terrana
```

One stable-ABI (`abi3`) wheel runs on CPython 3.9 and later. No Rust toolchain needed.

## Command line

The wheel installs a `terrana` command, the same as the Rust binary:

```bash
terrana serve observations.csv --lat latitude --lon longitude
# → REST API running at http://localhost:8080
curl "localhost:8080/query?lat=36.5&lon=-82.5&radius=10km"
```

Latitude and longitude columns are detected automatically when `--lat`/`--lon` are left out.
See the [API reference](https://github.com/jmcmeen/terrana#api-endpoints) for every endpoint:
radius, bounding-box and nearest-neighbour queries, point-in-polygon, filters, paging, and
geodesic geometry (area, buffer, convex hull, distance and more).

## Library mode (in-process, no server)

```python
import json
import terrana

session = terrana.load_csv("observations.csv")   # also load_parquet / load_geojson
print(session.row_count, session.lat_col, session.lon_col)

# Spatial queries take (lat, lon). Each row is a dict; radius and nearest add `_distance_km`.
rows = session.query_radius(36.54, -82.54, 5000)          # lat, lon, radius in metres
box  = session.query_bbox(35.0, -84.0, 37.0, -81.0)       # min_lat, min_lon, max_lat, max_lon
near = session.query_nearest(36.5, -82.5, 5)              # lat, lon, k

# Geodesic geometry takes (lon, lat), GeoJSON order. GeoJSON results are JSON strings.
d    = terrana.geodesic_distance(-82.54, 36.54, -82.55, 36.55)   # lon1, lat1, lon2, lat2
a    = terrana.geodesic_area('{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,1],[0,0]]]}')
hull = json.loads(terrana.convex_hull('{"type":"MultiPoint","coordinates":[[0,0],[1,0],[0,1]]}'))
buf  = json.loads(terrana.buffer(-82.5, 36.5, 5000, segments=64))  # lon, lat, metres
```

> **Coordinate order:** spatial queries are `(lat, lon)`, like the REST API; geometry functions
> are `(lon, lat)`, like GeoJSON.

Area and distance use Karney's geodesic algorithms on the WGS 84 ellipsoid, not planar math.

## Embedded server

Run the full REST API on a background thread:

```python
import os
import terrana

session = terrana.load_csv("observations.csv")

with session.serve_background(port=8080):
    ...  # http://localhost:8080 is live; it stops when the block ends

server = session.serve_background(port=8080)
server.shutdown()          # or manage it explicitly

# session.serve(port=8080) blocks in the foreground until Ctrl-C
```

Serving never holds Python's GIL. A port that's already taken raises `OSError`.

### Access control

On the default `127.0.0.1` only this machine can reach the server. Before binding anything
else, set a token; every endpoint except `/health` then needs
`Authorization: Bearer <token>`:

```python
session.serve_background(
    port=8080,
    bind="0.0.0.0",
    token=os.environ.get("TERRANA_TOKEN") or None,
    cors_origins=["https://maps.example.org"],   # default: any origin; ["none"] turns CORS off
)
```

Binding a non-loopback address without a token raises a `RuntimeWarning`. The `terrana`
command reads the token from `TERRANA_TOKEN` or `--token-file`.

## Links

- [Source and full documentation](https://github.com/jmcmeen/terrana)
- [Changelog](https://github.com/jmcmeen/terrana/blob/main/CHANGELOG.md)
- [Issues](https://github.com/jmcmeen/terrana/issues)
- [Rust crate](https://crates.io/crates/terrana)

Licensed under MIT or Apache-2.0, at your option.
