//! End-to-end API tests.
//!
//! Each test spawns the real `terrana` binary against the bundled `testdata/` and
//! exercises the HTTP endpoints over the loopback interface.
//!
//! These are `#[ignore]`d by default: starting the server builds a DuckDB spatial
//! index, and the `spatial` extension is fetched from the network on first use. Run
//! them in an environment with network access (CI does this) via:
//!
//! ```sh
//! cargo test --test api -- --include-ignored
//! ```

use std::net::TcpListener;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

/// A spawned server bound to an ephemeral port. Killed on drop.
struct TestServer {
    child: Child,
    port: u16,
}

impl TestServer {
    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{}", self.port, path)
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The response's `Content-Type` header as an owned string (empty if absent).
fn content_type(resp: &ureq::http::Response<ureq::Body>) -> String {
    resp.headers()
        .get(ureq::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string()
}

/// Grab a free TCP port by binding to :0 and releasing it.
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Spawn `terrana serve testdata/observations.csv` and wait for `/health` to answer.
fn spawn(extra_args: &[&str]) -> TestServer {
    spawn_file("testdata/observations.csv", extra_args)
}

/// Spawn `terrana serve <file>` and wait for `/health` to answer.
fn spawn_file(file: &str, extra_args: &[&str]) -> TestServer {
    let port = free_port();
    let port_str = port.to_string();
    let mut args = vec!["serve", file, "--port", &port_str];
    args.extend_from_slice(extra_args);

    let child = Command::new(env!("CARGO_BIN_EXE_terrana"))
        .args(&args)
        .spawn()
        .expect("failed to spawn terrana binary");
    let server = TestServer { child, port };

    // Connection is refused until the server binds, so failed calls return fast.
    // Allow a generous window for the one-time spatial-extension download.
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        assert!(
            Instant::now() < deadline,
            "server did not become ready within timeout"
        );
        if let Ok(resp) = ureq::get(server.url("/health")).call() {
            if resp.status().as_u16() == 200 {
                return server;
            }
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

/// Poll `/schema`'s `row_count` until it equals `want` or `timeout` elapses.
/// Returns the last value observed (for assertion messages).
fn wait_for_row_count(server: &TestServer, want: i64, timeout: Duration) -> i64 {
    let deadline = Instant::now() + timeout;
    loop {
        let last = get_json(server, "/schema")["row_count"]
            .as_i64()
            .unwrap_or(-1);
        if last == want || Instant::now() >= deadline {
            return last;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// GET helper returning the parsed JSON body, panicking on transport/status errors.
fn get_json(server: &TestServer, path: &str) -> serde_json::Value {
    ureq::get(server.url(path))
        .call()
        .unwrap_or_else(|e| panic!("GET {path} failed: {e}"))
        .body_mut()
        .read_json()
        .expect("response was not JSON")
}

#[test]
#[ignore = "requires network for the DuckDB spatial extension; run with --include-ignored"]
fn health_schema_stats() {
    let s = spawn(&[]);

    let health = get_json(&s, "/health");
    assert_eq!(health["status"], "ok");

    let schema = get_json(&s, "/schema");
    assert_eq!(schema["lat_column"], "latitude");
    assert_eq!(schema["lon_column"], "longitude");
    assert_eq!(schema["row_count"], 20);
    let col_names: Vec<&str> = schema["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert!(col_names.contains(&"species"));

    let stats = get_json(&s, "/stats");
    assert_eq!(stats["row_count"], 20);
    // testdata has valid coordinates, so the extent is populated (not null).
    assert!(stats["bbox"].is_array());
    assert!(stats["centroid"]["lat"].is_number());
}

#[test]
#[ignore = "requires network for the DuckDB spatial extension; run with --include-ignored"]
fn query_bbox_returns_rows() {
    let s = spawn(&[]);
    let rows = get_json(&s, "/query?bbox=35.0,-84.0,37.0,-81.0");
    let arr = rows.as_array().expect("expected a JSON array");
    assert!(!arr.is_empty(), "bbox query returned no rows");
}

#[test]
#[ignore = "requires network for the DuckDB spatial extension; run with --include-ignored"]
fn query_radius_sorted_by_distance() {
    let s = spawn(&[]);
    let rows = get_json(&s, "/query?lat=36.5&lon=-82.5&radius=500km");
    let arr = rows.as_array().expect("expected a JSON array");
    assert!(!arr.is_empty(), "radius query returned no rows");

    let mut prev = f64::MIN;
    for row in arr {
        let d = row["_distance_km"]
            .as_f64()
            .expect("each row should carry _distance_km");
        assert!(d >= prev, "rows must be sorted ascending by distance");
        prev = d;
    }
}

#[test]
#[ignore = "requires network for the DuckDB spatial extension; run with --include-ignored"]
fn query_nearest_limits_results() {
    let s = spawn(&[]);
    let rows = get_json(&s, "/query?lat=36.5&lon=-82.5&nearest=5");
    let arr = rows.as_array().expect("expected a JSON array");
    assert!(arr.len() <= 5);
    assert!(arr.iter().all(|r| r["_distance_km"].is_number()));
}

#[test]
#[ignore = "requires network for the DuckDB spatial extension; run with --include-ignored"]
fn query_select_restricts_columns() {
    let s = spawn(&[]);
    let rows = get_json(
        &s,
        "/query?bbox=35.0,-84.0,37.0,-81.0&select=species,observed_on",
    );
    for row in rows.as_array().unwrap() {
        let obj = row.as_object().unwrap();
        let mut keys: Vec<&String> = obj.keys().collect();
        keys.sort();
        assert_eq!(
            keys,
            vec![&"observed_on".to_string(), &"species".to_string()]
        );
    }
}

#[test]
#[ignore = "requires network for the DuckDB spatial extension; run with --include-ignored"]
fn query_csv_and_geojson_formats() {
    let s = spawn(&[]);

    let mut csv = ureq::get(s.url("/query?bbox=35.0,-84.0,37.0,-81.0&format=csv"))
        .call()
        .unwrap();
    assert!(content_type(&csv).starts_with("text/csv"));
    let body = csv.body_mut().read_to_string().unwrap();
    assert!(body.lines().next().unwrap().contains("species"));

    let mut geo = ureq::get(s.url("/query?bbox=35.0,-84.0,37.0,-81.0&format=geojson"))
        .call()
        .unwrap();
    assert!(content_type(&geo).contains("geo+json"));
    let fc: serde_json::Value = geo.body_mut().read_json().unwrap();
    assert_eq!(fc["type"], "FeatureCollection");
    assert!(fc["features"].is_array());
}

#[test]
#[ignore = "requires network for the DuckDB spatial extension; run with --include-ignored"]
fn within_polygon_query() {
    let s = spawn(&[]);
    let body = std::fs::read_to_string("testdata/parks.geojson").unwrap();
    let geojson: serde_json::Value = serde_json::from_str(&body).unwrap();

    let rows: serde_json::Value = ureq::post(s.url("/query/within"))
        .send_json(geojson)
        .unwrap()
        .body_mut()
        .read_json()
        .unwrap();
    assert!(rows.is_array(), "within should return a JSON array");
}

#[test]
#[ignore = "requires network for the DuckDB spatial extension; run with --include-ignored"]
fn geometry_area_unit_box() {
    let s = spawn(&[]);
    let req = serde_json::json!({
        "geometry": {
            "type": "Polygon",
            "coordinates": [[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [0.0, 0.0]]]
        }
    });
    let out: serde_json::Value = ureq::post(s.url("/geometry/area"))
        .send_json(req)
        .unwrap()
        .body_mut()
        .read_json()
        .unwrap();
    let area_km2 = out["area_km2"].as_f64().unwrap();
    assert!(
        (12_000.0..12_700.0).contains(&area_km2),
        "expected ~12,308 km², got {area_km2}"
    );
}

#[test]
#[ignore = "requires network for the DuckDB spatial extension; run with --include-ignored"]
fn geometry_distance_between_points() {
    let s = spawn(&[]);
    let req = serde_json::json!({
        "from": { "type": "Point", "coordinates": [-82.5, 36.5] },
        "to":   { "type": "Point", "coordinates": [-82.0, 36.0] }
    });
    let out: serde_json::Value = ureq::post(s.url("/geometry/distance"))
        .send_json(req)
        .unwrap()
        .body_mut()
        .read_json()
        .unwrap();
    assert!(out["distance_km"].as_f64().unwrap() > 0.0);
    assert!(out["bearing_deg"].is_number());
}

#[test]
#[ignore = "requires network for the DuckDB spatial extension; run with --include-ignored"]
fn invalid_select_column_is_rejected() {
    let s = spawn(&[]);
    // A select column with illegal characters must be rejected, not executed.
    let err = ureq::get(s.url("/query?bbox=35.0,-84.0,37.0,-81.0&select=a;b"))
        .call()
        .expect_err("expected a 400 error");
    match err {
        ureq::Error::StatusCode(code) => assert_eq!(code, 400),
        other => panic!("expected HTTP status error, got {other}"),
    }
}

#[test]
#[ignore = "requires network for the DuckDB spatial extension; run with --include-ignored"]
fn watch_reload_reflects_new_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("obs.csv");
    std::fs::write(&path, "id,latitude,longitude\n1,36.5,-82.5\n2,36.6,-82.4\n").unwrap();

    let s = spawn_file(path.to_str().unwrap(), &["--watch"]);
    assert_eq!(get_json(&s, "/schema")["row_count"], 2);

    // Rewrite the file with two more rows; --watch should re-ingest it.
    std::fs::write(
        &path,
        "id,latitude,longitude\n1,36.5,-82.5\n2,36.6,-82.4\n3,36.7,-82.3\n4,36.8,-82.2\n",
    )
    .unwrap();

    let got = wait_for_row_count(&s, 4, Duration::from_secs(30));
    assert_eq!(got, 4, "watch should re-ingest the new rows");
}

#[test]
#[ignore = "requires network for the DuckDB spatial extension; run with --include-ignored"]
fn watch_reload_keeps_old_data_on_bad_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("obs.csv");
    std::fs::write(&path, "id,latitude,longitude\n1,36.5,-82.5\n2,36.6,-82.4\n").unwrap();

    let s = spawn_file(path.to_str().unwrap(), &["--watch"]);
    assert_eq!(get_json(&s, "/schema")["row_count"], 2);

    // Overwrite with a file that loads fine but has no lat/lon columns — a deterministic
    // detection failure mid-reload. The atomic reload must keep serving the old data.
    std::fs::write(&path, "a,b,c\n1,2,3\n").unwrap();

    // Give the watcher time to fire and (fail to) reload, then assert nothing changed.
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(
        get_json(&s, "/schema")["row_count"],
        2,
        "old data must survive a failed reload"
    );
    assert_eq!(get_json(&s, "/health")["status"], "ok");

    // A subsequent good file must still reload — the watcher recovered from the failure.
    std::fs::write(
        &path,
        "id,latitude,longitude\n1,36.5,-82.5\n2,36.6,-82.4\n3,36.7,-82.3\n",
    )
    .unwrap();
    let got = wait_for_row_count(&s, 3, Duration::from_secs(30));
    assert_eq!(got, 3, "watcher should recover and reload a good file");
}
