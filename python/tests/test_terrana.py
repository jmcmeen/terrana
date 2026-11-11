"""End-to-end tests for the `terrana` Python bindings.

Covers both library mode (in-process queries + geodesic geometry) and server mode
(the embedded HTTP server) against the bundled ``testdata/observations.csv``.

Run with::

    uv run --with pytest pytest tests/ -v
"""

import csv
import json
import math
import os
import signal
import socket
import subprocess
import sys
import time
import urllib.error
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


def _true_distances(lat, lon):
    """(id, great-circle km) for every test row, nearest first, ties in file order
    (how the server breaks them). Haversine with the mean Earth radius, independent
    of DuckDB."""
    with open(DATA) as f:
        rows = list(csv.DictReader(f))
    out = []
    for r in rows:
        p1, p2 = math.radians(lat), math.radians(float(r["latitude"]))
        dl = math.radians(float(r["longitude"]) - lon)
        a = math.sin((p2 - p1) / 2) ** 2 + math.cos(p1) * math.cos(p2) * math.sin(dl / 2) ** 2
        out.append((int(r["id"]), 2 * 6371.0088 * math.asin(math.sqrt(a))))
    return sorted(out, key=lambda t: t[1])  # stable: keeps file order on ties


def test_query_nearest_distances_are_great_circle_km(session):
    want = _true_distances(36.5, -82.5)[:5]
    got = session.query_nearest(36.5, -82.5, 5)
    assert [r["id"] for r in got] == [i for i, _ in want]
    for r, (_, d) in zip(got, want):
        assert abs(r["_distance_km"] - d) < 1e-3


def test_query_radius_matches_great_circle_km(session):
    dists = _true_distances(36.5, -82.5)
    # 6.08 km falls between rows (none within 50 m), so membership is unambiguous.
    assert all(abs(d - 6.08) > 0.05 for _, d in dists)
    within = {i for i, d in dists if d <= 6.08}
    assert within
    assert {r["id"] for r in session.query_radius(36.5, -82.5, 6080)} == within


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


def _wait_healthy(port):
    deadline = time.time() + 15
    while time.time() < deadline:
        try:
            with _get(port, "/health", 1) as r:
                if r.status == 200:
                    return
        except Exception:
            pass
        time.sleep(0.3)
    raise AssertionError("embedded server did not become healthy")


def _status(port, path, headers=None, method="GET"):
    req = urllib.request.Request(
        f"http://127.0.0.1:{port}{path}", headers=headers or {}, method=method
    )
    # Headers come back as an email.message.Message: .get() ignores case.
    try:
        with urllib.request.urlopen(req, timeout=5) as r:
            return r.status, r.headers
    except urllib.error.HTTPError as e:
        e.close()
        return e.code, e.headers


def test_serve_background_with_a_token(session):
    port = _free_port()
    with session.serve_background(port=port, token="s3cret"):
        _wait_healthy(port)  # /health needs no token
        assert _status(port, "/schema")[0] == 401
        assert _status(port, "/schema", {"Authorization": "Bearer nope"})[0] == 401
        assert _status(port, "/schema", {"Authorization": "Bearer s3cret"})[0] == 200


def test_serve_background_with_cors_origins(session):
    port = _free_port()
    with session.serve_background(port=port, cors_origins=["https://maps.example.org"]):
        _wait_healthy(port)
        _, allowed = _status(port, "/health", {"Origin": "https://maps.example.org"})
        assert allowed.get("access-control-allow-origin") == "https://maps.example.org"
        _, other = _status(port, "/health", {"Origin": "https://other.example"})
        assert other.get("access-control-allow-origin") is None


@pytest.mark.parametrize(
    "kwargs",
    [
        {"token": ""},
        {"token": "   "},
        {"cors_origins": ["maps.example.org"]},
        {"cors_origins": []},
    ],
)
def test_serve_background_rejects_bad_settings(session, kwargs):
    with pytest.raises(ValueError):
        session.serve_background(port=_free_port(), **kwargs)


def test_a_public_bind_without_a_token_warns(session):
    port = _free_port()
    with pytest.warns(RuntimeWarning, match="without a token"):
        server = session.serve_background(port=port, bind="0.0.0.0")
    server.shutdown()


def test_a_public_bind_with_a_token_does_not_warn(session, recwarn):
    server = session.serve_background(port=_free_port(), bind="0.0.0.0", token="s3cret")
    server.shutdown()
    assert not [w for w in recwarn if issubclass(w.category, RuntimeWarning)]


def test_serve_background_raises_when_the_port_is_taken(session):
    taken = socket.socket()
    taken.bind(("127.0.0.1", 0))
    taken.listen()
    try:
        with pytest.raises(OSError):
            session.serve_background(port=taken.getsockname()[1])
    finally:
        taken.close()


def test_serve_background_explicit_shutdown(session):
    port = _free_port()
    server = session.serve_background(port=port)
    try:
        time.sleep(0.6)
        assert json.loads(_get(port, "/health", 2).read())["status"] == "ok"
    finally:
        server.shutdown()
        server.shutdown()  # idempotent


def test_serve_background_drop_stops_server(session):
    # Dropping the handle without shutdown() or the context manager must still stop
    # the server — Drop signals the runtime and joins the thread.
    port = _free_port()
    server = session.serve_background(port=port)
    deadline = time.time() + 15
    healthy = False
    while time.time() < deadline:
        try:
            if _get(port, "/health", 1).status == 200:
                healthy = True
                break
        except Exception:
            time.sleep(0.3)
    assert healthy, "embedded server did not become healthy"
    del server  # no shutdown(), no context manager
    with pytest.raises(Exception):
        _get(port, "/health", 1)


# --- CLI (the `terrana` console script installed by the wheel) ---

# The console script lives next to the interpreter that installed the wheel.
CLI = os.path.join(os.path.dirname(sys.executable), "terrana")

cli_required = pytest.mark.skipif(
    not os.path.exists(CLI), reason="terrana console script not installed in this env"
)


@cli_required
def test_cli_version():
    out = subprocess.run([CLI, "--version"], capture_output=True, text=True, timeout=30)
    assert out.returncode == 0
    assert out.stdout.startswith("terrana ")


@cli_required
def test_cli_serve_and_health():
    """`terrana serve` runs the same server as the binary, and SIGINT stops it."""
    port = _free_port()
    # Discard the server's logs (don't PIPE — an unread pipe could fill and stall the
    # child under request-trace logging).
    proc = subprocess.Popen(
        [CLI, "serve", DATA, "--port", str(port)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    try:
        deadline = time.time() + 30
        healthy = False
        while time.time() < deadline:
            if proc.poll() is not None:
                raise AssertionError("terrana serve exited before becoming healthy")
            try:
                if json.loads(_get(port, "/health", 1).read())["status"] == "ok":
                    healthy = True
                    break
            except Exception:
                time.sleep(0.3)
        assert healthy, "terrana serve did not become healthy"
        rows = json.loads(_get(port, "/query?bbox=35.0,-84.0,37.0,-81.0", 5).read())
        assert len(rows) > 0
    finally:
        # The console script resets SIGINT to the OS default, so Ctrl-C terminates the
        # process just like the Rust binary (rather than being swallowed by Python).
        proc.send_signal(signal.SIGINT)
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait()
            raise AssertionError("terrana serve did not exit on SIGINT")
