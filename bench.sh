#!/usr/bin/env bash
set -euo pipefail

# Terrana benchmark script
# Usage: ./bench.sh [10k|100k|1m|250m] [port] [--disk]
#
# Starts a terrana server against the specified dataset,
# runs a suite of timed queries, and prints a summary table.

DATASET="${1:-100k}"
PORT="${2:-9090}"
EXTRA_FLAGS="${3:-}"
FILE="testdata/bench_${DATASET}.csv"
BASE="http://localhost:${PORT}"
BINARY="./target/release/terrana"

if [[ ! -f "$FILE" ]]; then
    echo "Dataset not found: $FILE"
    if [[ "$DATASET" == "250m" ]]; then
        echo "Run: python3 testdata/generate_250m.py"
    else
        echo "Run: python3 testdata/generate_benchdata.py"
    fi
    exit 1
fi

if [[ ! -f "$BINARY" ]]; then
    echo "Release binary not found. Building..."
    cargo build --release
fi

# Start server in background
echo "Starting terrana on port $PORT with $FILE..."
$BINARY serve "$FILE" --port "$PORT" $EXTRA_FLAGS &
SERVER_PID=$!
trap "kill $SERVER_PID 2>/dev/null; wait $SERVER_PID 2>/dev/null" EXIT

# Wait for server to be ready
for i in $(seq 1 30); do
    if curl -s "$BASE/health" >/dev/null 2>&1; then
        break
    fi
    sleep 0.5
done

# Grab startup stats
STATS=$(curl -s "$BASE/stats")
INDEX_SIZE=$(echo "$STATS" | python3 -c "import sys,json; print(json.load(sys.stdin)['index_size'])")
BUILD_MS=$(echo "$STATS" | python3 -c "import sys,json; print(json.load(sys.stdin)['index_build_ms'])")

echo ""
echo "=== Terrana Benchmark ==="
echo "Dataset:     $FILE"
echo "Index size:  $INDEX_SIZE points"
echo "Index build: ${BUILD_MS}ms"
echo ""

# Benchmark runner — runs a curl, captures wall time and row count
results=()

run_bench() {
    local label="$1"
    local url="$2"
    local method="${3:-GET}"
    local body="${4:-}"

    if [[ "$method" == "POST" ]]; then
        local start=$(date +%s%N)
        local output=$(curl -s -X POST "$url" -H "Content-Type: application/json" -d "$body")
        local end=$(date +%s%N)
    else
        local start=$(date +%s%N)
        local output=$(curl -s "$url")
        local end=$(date +%s%N)
    fi

    local elapsed_ms=$(( (end - start) / 1000000 ))
    local count=$(echo "$output" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    if isinstance(d, list):
        print(len(d))
    elif 'features' in d:
        print(len(d['features']))
    elif 'area_km2' in d:
        print('1 result')
    elif 'distance_km' in d:
        print('1 result')
    else:
        print('ok')
except:
    print('error')
" 2>/dev/null)

    printf "  %-40s %8s  %8sms\n" "$label" "$count" "$elapsed_ms"
    results+=("$label|$count|${elapsed_ms}ms")
}

echo "--- Spatial Queries ---"
run_bench "Nearest 1"                "$BASE/query?lat=35.6&lon=-83.5&nearest=1"
run_bench "Nearest 10"               "$BASE/query?lat=35.6&lon=-83.5&nearest=10"
run_bench "Nearest 100"              "$BASE/query?lat=35.6&lon=-83.5&nearest=100"
run_bench "Radius 1km"               "$BASE/query?lat=35.6&lon=-83.5&radius=1km"
run_bench "Radius 5km"               "$BASE/query?lat=35.6&lon=-83.5&radius=5km"
run_bench "Radius 10km"              "$BASE/query?lat=35.6&lon=-83.5&radius=10km"
run_bench "Radius 50km (limit 10k)"  "$BASE/query?lat=35.6&lon=-83.5&radius=50km&limit=10000"
run_bench "BBox small (0.2°)"        "$BASE/query?bbox=35.5,-83.6,35.7,-83.4"
run_bench "BBox medium (1°)"         "$BASE/query?bbox=35.0,-84.0,36.0,-83.0&limit=5000"
run_bench "BBox large (2°)"          "$BASE/query?bbox=35.0,-84.0,37.0,-82.0&limit=10000"

echo ""
echo "--- Filters ---"
run_bench "Where (research only)"    "$BASE/query?bbox=35.0,-84.0,36.0,-82.0&where=quality_grade:research&limit=5000"
run_bench "Select 2 cols"            "$BASE/query?bbox=35.5,-83.6,35.7,-83.4&select=species,observed_on"
run_bench "Where + Select"           "$BASE/query?bbox=35.0,-84.0,36.0,-82.0&select=species,count&where=quality_grade:research&limit=5000"
run_bench "Group by + count"         "$BASE/query?bbox=35.0,-84.0,36.0,-82.0&group_by=species&agg=count&limit=100"
run_bench "Group by + sum"           "$BASE/query?bbox=35.0,-84.0,36.0,-82.0&group_by=species&agg=sum:count&limit=100"

echo ""
echo "--- Output Formats ---"
run_bench "JSON (1k rows)"           "$BASE/query?bbox=35.5,-83.6,35.7,-83.4&format=json"
run_bench "CSV (1k rows)"            "$BASE/query?bbox=35.5,-83.6,35.7,-83.4&format=csv"
run_bench "GeoJSON (1k rows)"        "$BASE/query?bbox=35.5,-83.6,35.7,-83.4&format=geojson"

echo ""
echo "--- Point-in-Polygon ---"
run_bench "Within (small poly)" \
    "$BASE/query/within" POST \
    '{"type":"Polygon","coordinates":[[[-83.6,35.5],[-83.4,35.5],[-83.4,35.7],[-83.6,35.7],[-83.6,35.5]]]}'
run_bench "Within (large poly)" \
    "$BASE/query/within" POST \
    '{"type":"Polygon","coordinates":[[[-84.0,35.0],[-82.0,35.0],[-82.0,36.5],[-84.0,36.5],[-84.0,35.0]]]}'

echo ""
echo "--- Geometry ---"
run_bench "Area (polygon)" \
    "$BASE/geometry/area" POST \
    '{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,1],[0,0]]]}'
run_bench "Distance (2 points)" \
    "$BASE/geometry/distance" POST \
    '{"from":{"type":"Point","coordinates":[-83.5,35.6]},"to":{"type":"Point","coordinates":[-82.5,36.5]}}'
run_bench "Buffer (1km)" \
    "$BASE/geometry/buffer" POST \
    '{"geometry":{"type":"Point","coordinates":[-83.5,35.6]},"distance":1000,"unit":"m","segments":64}'
run_bench "Centroid" \
    "$BASE/geometry/centroid" POST \
    '{"type":"Polygon","coordinates":[[[-83.6,35.5],[-83.4,35.5],[-83.4,35.7],[-83.6,35.7],[-83.6,35.5]]]}'
run_bench "Bounds" \
    "$BASE/geometry/bounds" POST \
    '{"type":"Polygon","coordinates":[[[-83.6,35.5],[-83.4,35.5],[-83.4,35.7],[-83.6,35.7],[-83.6,35.5]]]}'

echo ""
echo "--- Metadata ---"
run_bench "Health"  "$BASE/health"
run_bench "Schema"  "$BASE/schema"
run_bench "Stats"   "$BASE/stats"

echo ""
echo "=== Done ==="
