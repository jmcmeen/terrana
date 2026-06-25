"""End-to-end tests for the `terrana` Python bindings.

Covers both library mode (in-process queries + geodesic geometry) and server mode
(the embedded HTTP server) against the bundled ``testdata/observations.csv``.

Run with::

    uv run --with pytest pytest tests/ -v
"""

import json
import os
import socket
import time
import urllib.request

import pytest

import terrana

DATA = os.path.join(os.path.dirname(__file__), "..", "..", "testdata", "observations.csv")


@pytest.fixture
def session():
    return terrana.load_csv(DATA)


def _free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def _get(port, path, timeout=2):
    return urllib.request.urlopen(f"http://127.0.0.1:{port}{path}", timeout=timeout)


# --- Library mode ---


def test_load_reports_schema(session):
    assert session.row_count == 20
    assert session.lat_col == "latitude"
    assert session.lon_col == "longitude"


def test_query_radius(session):
    rows = session.query_radius(36.54, -82.54, 5000)
    assert len(rows) > 0
    assert all("_distance_km" in r for r in rows)


def test_query_bbox(session):
    rows = session.query_bbox(35.0, -84.0, 37.0, -81.0)
    assert len(rows) > 0


def test_query_nearest_sorted(session):
    rows = session.query_nearest(36.5, -82.5, 5)
    assert 0 < len(rows) <= 5
    dists = [r["_distance_km"] for r in rows]
    assert dists == sorted(dists)


def test_geodesic_distance():
    d = terrana.geodesic_distance(-82.54, 36.54, -82.55, 36.55)
    assert d["distance_m"] > 0
    assert "bearing_deg" in d


def test_geodesic_area():
    # A 1° x 1° box at the equator is ~12,308 km².
    area = terrana.geodesic_area(
        '{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,1],[0,0]]]}'
    )
    assert 12_000 < area["area_km2"] < 12_700


def test_convex_hull():
    hull = json.loads(
        terrana.convex_hull(
            '{"type":"MultiPoint","coordinates":[[0,0],[1,0],[1,1],[0,1],[0.5,0.5]]}'
        )
    )
    assert hull["type"] == "Feature"
    assert hull["geometry"]["type"] == "Polygon"
    assert hull["properties"]["point_count"] == 5


def test_buffer_area_is_disk():
    # A 5 km buffer is ~78 km² — guards against the clockwise-ring regression that
    # reported the whole Earth's surface.
    buf = json.loads(terrana.buffer(-82.5, 36.5, 5000, 64))
    assert 70 < buf["properties"]["area_km2"] < 85
    assert buf["properties"]["segments"] == 64


# --- Server mode ---


def test_serve_background_context_manager(session):
    port = _free_port()
    with session.serve_background(port=port):
        deadline = time.time() + 15
        healthy = False
        while time.time() < deadline:
            try:
                r = _get(port, "/health", 1)
                if r.status == 200 and json.loads(r.read())["status"] == "ok":
                    healthy = True
                    break
            except Exception:
                time.sleep(0.3)
        assert healthy, "embedded server did not become healthy"
        rows = json.loads(_get(port, "/query?bbox=35.0,-84.0,37.0,-81.0", 5).read())
        assert len(rows) > 0
    # Server is shut down on context exit — the port should refuse connections.
    with pytest.raises(Exception):
        _get(port, "/health", 1)


def test_serve_background_explicit_shutdown(session):
    port = _free_port()
    server = session.serve_background(port=port)
    try:
        time.sleep(0.6)
        assert json.loads(_get(port, "/health", 2).read())["status"] == "ok"
    finally:
        server.shutdown()
        server.shutdown()  # idempotent
