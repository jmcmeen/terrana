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

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
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

/// Spawn `terrana serve testdata/observations.csv` and wait for `/health` to answer.
fn spawn(extra_args: &[&str]) -> TestServer {
    spawn_file("testdata/observations.csv", extra_args)
}

/// Spawn `terrana serve <file> --port 0` and wait for `/health` to answer.
///
/// The OS picks the port when the server binds, and the server logs the address it
/// bound, so parallel tests can never be handed the same port.
fn spawn_file(file: &str, extra_args: &[&str]) -> TestServer {
    spawn_with_env(file, extra_args, &[])
}

/// [`spawn_file`] with extra environment variables for the server.
fn spawn_with_env(file: &str, extra_args: &[&str], envs: &[(&str, &str)]) -> TestServer {
    let mut args = vec!["serve", file, "--port", "0"];
    args.extend_from_slice(extra_args);
    let mut child = Command::new(env!("CARGO_BIN_EXE_terrana"))
        .args(&args)
        .env("NO_COLOR", "1")
        .env_remove("TERRANA_TOKEN")
        .envs(envs.iter().copied())
        .stdout(Stdio::piped())
        .spawn()
        .expect("failed to spawn terrana binary");

    // Read the log (stdout) on a thread: it must keep draining, or the server
    // blocks on a full pipe. Echo it for test output and send back the bound port.
    let stdout = child.stdout.take().expect("stdout is piped");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            println!("{line}");
            if let Some(port) = line
                .split("listening on ")
                .nth(1)
                .and_then(|addr| addr.trim().rsplit(':').next())
                .and_then(|p| p.parse::<u16>().ok())
            {
                let _ = tx.send(port);
            }
        }
    });

    // Allow a generous window for the one-time spatial-extension download.
    let port = rx
        .recv_timeout(Duration::from_secs(120))
        .expect("server did not report a listening port within timeout");
    let server = TestServer { child, port };

    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        assert!(
            Instant::now() < deadline,
            "server did not answer /health within timeout"
        );
        if let Ok(resp) = ureq::get(server.url("/health")).call() {
            if resp.status().as_u16() == 200 {
                return server;
            }
        }
        std::thread::sleep(Duration::from_millis(100));
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

/// GET helper returning the parsed JSON body and the `Link` header (if any).
fn get_page(server: &TestServer, path: &str) -> (Vec<serde_json::Value>, Option<String>) {
    let mut resp = ureq::get(server.url(path))
        .call()
        .unwrap_or_else(|e| panic!("GET {path} failed: {e}"));
    let link = resp
        .headers()
        .get(ureq::http::header::LINK)
        .map(|v| v.to_str().unwrap().to_string());
    let rows: serde_json::Value = resp.body_mut().read_json().expect("response was not JSON");
    (
        rows.as_array().expect("expected a JSON array").clone(),
        link,
    )
}

/// The path a `Link: <?query>; rel="next"` header points at, resolved against
/// `/query` (the target is a relative reference to the request's own path).
fn next_path(link: &str) -> String {
    assert!(link.ends_with("; rel=\"next\""), "unexpected Link: {link}");
    let target = link.trim_start_matches('<').split('>').next().unwrap();
    assert!(
        target.starts_with('?'),
        "expected a relative ?query target: {link}"
    );
    format!("/query{target}")
}

#[test]
#[ignore = "requires network for the DuckDB spatial extension; run with --include-ignored"]
fn query_pages_follow_the_link_header_through_every_row() {
    let s = spawn(&[]);
    let (all, link) = get_page(&s, "/query?bbox=35.0,-84.0,37.0,-81.0");
    assert!(link.is_none(), "one page holds everything: no next link");
    assert!(all.len() > 7, "test data should span several pages of 3");

    let mut paged = Vec::new();
    let mut path = Some("/query?bbox=35.0,-84.0,37.0,-81.0&limit=3".to_string());
    while let Some(p) = path {
        let (rows, link) = get_page(&s, &p);
        assert!(rows.len() <= 3);
        paged.extend(rows);
        path = link.map(|l| next_path(&l));
    }
    assert_eq!(paged, all, "pages must cover every row once, in order");
}

#[test]
#[ignore = "requires network for the DuckDB spatial extension; run with --include-ignored"]
fn query_pages_radius_results_in_distance_order() {
    let s = spawn(&[]);
    let (all, _) = get_page(&s, "/query?lat=36.5&lon=-82.5&radius=500km");
    assert!(all.len() >= 8, "test data should have 8 rows within 500 km");
    let (first, link) = get_page(&s, "/query?lat=36.5&lon=-82.5&radius=500km&limit=4");
    let (second, _) = get_page(&s, &next_path(&link.expect("more rows follow")));
    assert_eq!(first, all[..4]);
    assert_eq!(second, all[4..8]);
}

#[test]
#[ignore = "requires network for the DuckDB spatial extension; run with --include-ignored"]
fn query_nearest_with_offset_returns_the_next_nearest() {
    let s = spawn(&[]);
    let (ten, _) = get_page(&s, "/query?lat=36.5&lon=-82.5&nearest=10");
    assert_eq!(ten.len(), 10, "test data should have 10 rows");
    let (next, link) = get_page(&s, "/query?lat=36.5&lon=-82.5&nearest=5&offset=5");
    assert_eq!(next, ten[5..10]);
    assert_eq!(
        next_path(&link.expect("more rows follow")),
        "/query?lat=36.5&lon=-82.5&nearest=5&offset=10"
    );
}

#[test]
#[ignore = "requires network for the DuckDB spatial extension; run with --include-ignored"]
fn query_offset_past_the_end_is_empty_and_bad_offsets_are_rejected() {
    let s = spawn(&[]);
    let (rows, link) = get_page(&s, "/query?bbox=35.0,-84.0,37.0,-81.0&offset=100000");
    assert!(rows.is_empty());
    assert!(link.is_none());

    for bad in ["-1", "ten", "1.5"] {
        let err = ureq::get(s.url(&format!("/query?offset={bad}")))
            .call()
            .expect_err("expected a 400 error");
        match err {
            ureq::Error::StatusCode(code) => assert_eq!(code, 400, "offset={bad}"),
            other => panic!("expected HTTP status error, got {other}"),
        }
    }
}

#[test]
#[ignore = "requires network for the DuckDB spatial extension; run with --include-ignored"]
fn query_pages_csv_with_the_same_link_header() {
    let s = spawn(&[]);
    let resp = ureq::get(s.url("/query?bbox=35.0,-84.0,37.0,-81.0&format=csv&limit=2"))
        .call()
        .unwrap();
    let link = resp
        .headers()
        .get(ureq::http::header::LINK)
        .expect("more rows follow")
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(
        next_path(&link),
        "/query?bbox=35.0,-84.0,37.0,-81.0&format=csv&limit=2&offset=2"
    );
}

#[test]
#[ignore = "requires network for the DuckDB spatial extension; run with --include-ignored"]
fn query_pages_aggregates_by_group() {
    let s = spawn(&[]);
    let (all, _) = get_page(&s, "/query?group_by=species&agg=count");
    assert!(all.len() >= 4, "test data should have 4 species");
    let (first, link) = get_page(&s, "/query?group_by=species&agg=count&limit=2");
    let (second, _) = get_page(&s, &next_path(&link.expect("more groups follow")));
    assert_eq!([first, second].concat(), all[..4]);
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
fn geometry_buffer_area_is_sane() {
    let s = spawn(&[]);
    let req = serde_json::json!({
        "geometry": { "type": "Point", "coordinates": [-82.5, 36.5] },
        "distance": 5, "unit": "km", "segments": 64
    });
    let out: serde_json::Value = ureq::post(s.url("/geometry/buffer"))
        .send_json(req)
        .unwrap()
        .body_mut()
        .read_json()
        .unwrap();
    // A 5 km buffer is ~78 km². A clockwise-wound ring would report Earth's whole
    // surface (~5.1e8 km²) — this guards against that regression.
    let area_km2 = out["properties"]["area_km2"].as_f64().unwrap();
    assert!(
        (70.0..85.0).contains(&area_km2),
        "expected ~78 km², got {area_km2}"
    );
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

/// The `id`s a `/query` returns, in order.
fn ids(server: &TestServer, path: &str) -> Vec<i64> {
    get_json(server, path)
        .as_array()
        .expect("expected a JSON array")
        .iter()
        .map(|r| r["id"].as_i64().expect("each row has an id"))
        .collect()
}

/// The HTTP status of a request expected to fail.
fn error_status(server: &TestServer, path: &str) -> u16 {
    match ureq::get(server.url(path)).call() {
        Err(ureq::Error::StatusCode(code)) => code,
        Ok(_) => panic!("GET {path} should have failed"),
        Err(other) => panic!("expected HTTP status error, got {other}"),
    }
}

#[test]
#[ignore = "requires network for the DuckDB spatial extension; run with --include-ignored"]
fn where_compares_with_the_column_type() {
    let s = spawn(&[]);
    // count is BIGINT, observed_on a DATE (testdata/observations.csv).
    assert_eq!(ids(&s, "/query?where=count:gte:6&select=id"), [5, 13, 15]);
    assert_eq!(ids(&s, "/query?where=count:gt:7&select=id"), [5]);
    assert_eq!(
        ids(&s, "/query?where=count:between:6|7&select=id"),
        [13, 15]
    );
    assert_eq!(
        ids(&s, "/query?where=observed_on:gte:2024-03-24&select=id"),
        [19, 20]
    );
    assert_eq!(
        ids(&s, "/query?where=observed_on:lt:2024-03-16&select=id"),
        [1, 2]
    );
    // Column names match whatever their case, as in DuckDB.
    assert_eq!(ids(&s, "/query?where=COUNT:gte:6&select=id"), [5, 13, 15]);
    // The original column:value form still matches.
    assert_eq!(ids(&s, "/query?where=count:8&select=id"), [5]);
    assert_eq!(
        ids(
            &s,
            "/query?where=count:ne:1,count:ne:2,count:lt:4&select=id"
        ),
        [1, 18]
    );
}

#[test]
#[ignore = "requires network for the DuckDB spatial extension; run with --include-ignored"]
fn where_in_and_not_in_lists() {
    let s = spawn(&[]);
    assert_eq!(
        ids(
            &s,
            "/query?where=species:in:Barred%20Owl|Black%20Bear&select=id"
        ),
        [8, 9]
    );
    let casual_or_needs_id = ids(&s, "/query?where=quality_grade:nin:research&select=id");
    let research = ids(&s, "/query?where=quality_grade:research&select=id");
    assert_eq!(casual_or_needs_id.len() + research.len(), 20);
    assert!(casual_or_needs_id.iter().all(|i| !research.contains(i)));
}

#[test]
#[ignore = "requires network for the DuckDB spatial extension; run with --include-ignored"]
fn repeated_where_parameters_all_apply() {
    let s = spawn(&[]);
    // Eastern Bluebird (id 1) is research grade, so both filters together match nothing.
    assert_eq!(
        ids(&s, "/query?where=species:Eastern%20Bluebird&select=id"),
        [1]
    );
    assert!(ids(
        &s,
        "/query?where=quality_grade:casual&where=species:Eastern%20Bluebird&select=id"
    )
    .is_empty());
    assert_eq!(
        ids(
            &s,
            "/query?where=quality_grade:research&where=count:gte:6&select=id"
        ),
        [5, 15]
    );
}

#[test]
#[ignore = "requires network for the DuckDB spatial extension; run with --include-ignored"]
fn where_null_tests() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("obs.csv");
    std::fs::write(
        &path,
        "id,latitude,longitude,count\n1,36.5,-82.5,3\n2,36.6,-82.4,\n3,36.7,-82.3,5\n",
    )
    .unwrap();
    let s = spawn_file(path.to_str().unwrap(), &[]);
    assert_eq!(ids(&s, "/query?where=count:isnull&select=id"), [2]);
    assert_eq!(ids(&s, "/query?where=count:notnull&select=id"), [1, 3]);
    // Like SQL, ne doesn't match nulls.
    assert_eq!(ids(&s, "/query?where=count:ne:3&select=id"), [3]);
}

#[test]
#[ignore = "requires network for the DuckDB spatial extension; run with --include-ignored"]
fn where_rejects_bad_values_and_unknown_columns() {
    let s = spawn(&[]);
    for path in [
        "/query?where=count:gt:many",
        "/query?where=observed_on:gte:not-a-date",
        "/query?where=count:between:5",
        "/query?where=nosuch:1",
        "/query?bbox=35.0,-84.0,37.0,-81.0&where=nosuch:1",
        "/query?group_by=species&agg=count&where=nosuch:1",
    ] {
        assert_eq!(error_status(&s, path), 400, "{path}");
    }
}

#[test]
#[ignore = "requires network for the DuckDB spatial extension; run with --include-ignored"]
fn a_token_guards_everything_but_health() {
    let s = spawn_with_env(
        "testdata/observations.csv",
        &[],
        &[("TERRANA_TOKEN", "s3cret")],
    );
    assert_eq!(get_json(&s, "/health")["status"], "ok");

    for path in ["/query?limit=1", "/schema", "/stats"] {
        assert_eq!(error_status(&s, path), 401, "{path} without a token");
        let wrong = ureq::get(s.url(path))
            .header("Authorization", "Bearer nope")
            .call();
        assert!(
            matches!(wrong, Err(ureq::Error::StatusCode(401))),
            "{path} with a wrong token"
        );
        let resp = ureq::get(s.url(path))
            .header("Authorization", "Bearer s3cret")
            .call()
            .unwrap_or_else(|e| panic!("{path} with the token: {e}"));
        assert_eq!(resp.status().as_u16(), 200);
    }
    // The scheme is case-insensitive.
    let lower = ureq::get(s.url("/schema"))
        .header("Authorization", "bearer s3cret")
        .call()
        .unwrap();
    assert_eq!(lower.status().as_u16(), 200);
    // Default CORS with a token: a browser preflight asking for Authorization is allowed.
    let preflight = ureq::options(s.url("/query"))
        .header("Origin", "https://any.example")
        .header("Access-Control-Request-Method", "GET")
        .header("Access-Control-Request-Headers", "authorization")
        .call()
        .expect("preflight should succeed without a token");
    assert!(header_of(&preflight, "access-control-allow-headers")
        .unwrap()
        .contains("authorization"));
    let post = ureq::post(s.url("/geometry/distance")).send_json(serde_json::json!({
        "from": {"type": "Point", "coordinates": [0.0, 0.0]},
        "to": {"type": "Point", "coordinates": [0.0, 1.0]}
    }));
    assert!(matches!(post, Err(ureq::Error::StatusCode(401))));
}

#[test]
#[ignore = "requires network for the DuckDB spatial extension; run with --include-ignored"]
fn the_token_can_come_from_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let token = dir.path().join("token");
    std::fs::write(&token, "from-file\n").unwrap();
    let s = spawn_file(
        "testdata/observations.csv",
        &["--token-file", token.to_str().unwrap()],
    );
    assert_eq!(error_status(&s, "/schema"), 401);
    let resp = ureq::get(s.url("/schema"))
        .header("Authorization", "Bearer from-file")
        .call()
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
}

/// The value of response header `name`, if present.
fn header_of(resp: &ureq::http::Response<ureq::Body>, name: &str) -> Option<String> {
    resp.headers()
        .get(name)
        .map(|v| v.to_str().unwrap().to_string())
}

#[test]
#[ignore = "requires network for the DuckDB spatial extension; run with --include-ignored"]
fn cors_allows_only_the_listed_origins() {
    let s = spawn_with_env(
        "testdata/observations.csv",
        &["--cors-origin", "https://maps.example.org"],
        &[("TERRANA_TOKEN", "s3cret")],
    );
    // A browser preflight carries no token, and must still get through.
    let preflight = ureq::options(s.url("/query"))
        .header("Origin", "https://maps.example.org")
        .header("Access-Control-Request-Method", "GET")
        .header("Access-Control-Request-Headers", "authorization")
        .call()
        .expect("preflight should succeed without a token");
    assert_eq!(
        header_of(&preflight, "access-control-allow-origin").as_deref(),
        Some("https://maps.example.org")
    );
    assert!(header_of(&preflight, "access-control-allow-headers")
        .unwrap()
        .contains("authorization"));

    let get = |origin: &str| {
        ureq::get(s.url("/query?limit=1"))
            .header("Origin", origin)
            .header("Authorization", "Bearer s3cret")
            .call()
            .unwrap()
    };
    let allowed = get("https://maps.example.org");
    assert_eq!(
        header_of(&allowed, "access-control-allow-origin").as_deref(),
        Some("https://maps.example.org")
    );
    assert_eq!(
        header_of(&allowed, "access-control-expose-headers").as_deref(),
        Some("link")
    );
    assert_eq!(
        header_of(&get("https://other.example"), "access-control-allow-origin"),
        None
    );
}

#[test]
#[ignore = "requires network for the DuckDB spatial extension; run with --include-ignored"]
fn cors_is_open_by_default_and_can_be_turned_off() {
    let open = spawn(&[]);
    let resp = ureq::get(open.url("/health"))
        .header("Origin", "https://any.example")
        .call()
        .unwrap();
    assert_eq!(
        header_of(&resp, "access-control-allow-origin").as_deref(),
        Some("*")
    );

    let off = spawn(&["--cors-origin", "none"]);
    let resp = ureq::get(off.url("/health"))
        .header("Origin", "https://any.example")
        .call()
        .unwrap();
    assert_eq!(header_of(&resp, "access-control-allow-origin"), None);
}
