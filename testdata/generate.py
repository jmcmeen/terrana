#!/usr/bin/env python3
"""Generate benchmark CSV files with simulated Southern Appalachian wildlife observations.

Single source of truth for Terrana's benchmark data. Uses a fast raw-string +
buffered-write path for every size (faster than csv.writer, and safe here since
no field needs CSV quoting).

Usage:
    python3 testdata/generate.py --out FILE --rows N [--seed 42]
    python3 testdata/generate.py --preset bench   # bench_10k/100k/1m.csv
    python3 testdata/generate.py --preset 250m    # bench_250m.csv (~15 GB)
"""

import argparse
import os
import random
import time
from datetime import date, timedelta

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))

CHUNK_SIZE = 100_000  # rows per write buffer
REPORT_EVERY = 1_000_000

# --- Species list: ~30 realistic Southern Appalachian species ---
SPECIES = [
    # Birds
    "Eastern Bluebird", "Carolina Chickadee", "Red-tailed Hawk",
    "Pileated Woodpecker", "Scarlet Tanager", "Wood Thrush",
    "Cerulean Warbler", "Peregrine Falcon", "Barred Owl",
    "Ruffed Grouse", "Wild Turkey", "Dark-eyed Junco",
    # Mammals
    "White-tailed Deer", "Black Bear", "Eastern Chipmunk",
    "Gray Squirrel", "Red Fox", "Coyote", "Bobcat",
    "Northern Flying Squirrel", "Elk",
    # Reptiles
    "Eastern Box Turtle", "Timber Rattlesnake", "Eastern Fence Lizard",
    "Northern Copperhead",
    # Amphibians
    "Eastern Hellbender", "Red-spotted Newt", "Jordan's Salamander",
    "Spring Peeper",
    # Plants
    "Flame Azalea", "Fraser Fir",
]

QUALITY_GRADES = ["research", "casual", "needs_id"]
QUALITY_WEIGHTS = [60, 25, 15]  # research / casual / needs_id, out of 100

# --- Hotspot clusters (lat, lon, std_dev) ---
# Great Smoky Mountains NP, Blue Ridge Parkway, Pisgah / Nantahala / Cherokee NF.
HOTSPOTS = [
    (35.61, -83.53, 0.15),   # Great Smoky Mtns - Clingmans Dome area
    (35.63, -83.20, 0.12),   # Great Smoky Mtns - east side
    (35.70, -82.50, 0.18),   # Blue Ridge Parkway - Asheville area
    (36.05, -81.80, 0.14),   # Blue Ridge Parkway - Blowing Rock area
    (36.60, -81.40, 0.12),   # Blue Ridge Parkway - VA section
    (35.30, -82.80, 0.13),   # Pisgah National Forest
    (35.20, -83.55, 0.11),   # Nantahala National Forest
    (35.10, -84.10, 0.10),   # Cherokee NF - south
    (36.10, -82.40, 0.12),   # Cherokee NF - north
    (35.78, -83.00, 0.09),   # Max Patch / AT corridor
]

LAT_MIN, LAT_MAX = 34.5, 37.0
LON_MIN, LON_MAX = -84.5, -81.0

# Pre-compute all dates as strings (2023-01-01 to 2024-12-31 = 731 days)
_DATE_START = date(2023, 1, 1)
_DATE_END = date(2024, 12, 31)
_NUM_DAYS = (_DATE_END - _DATE_START).days
DATES = [(_DATE_START + timedelta(days=d)).isoformat() for d in range(_NUM_DAYS + 1)]

# Pre-compute count distribution: favor low values (P proportional to 1/x, 1-20).
_count_weights = [1.0 / x for x in range(1, 21)]
_count_total = sum(_count_weights)
_COUNT_CUM = []
_c = 0.0
for _w in _count_weights:
    _c += _w / _count_total
    _COUNT_CUM.append(_c)


def pick_count(r):
    """r is a random float [0,1)."""
    for i, threshold in enumerate(_COUNT_CUM):
        if r <= threshold:
            return i + 1
    return 20


def generate(filepath, num_rows, seed=42):
    name = os.path.basename(filepath)
    rng = random.Random(seed)  # deterministic for reproducibility
    gauss = rng.gauss
    uniform = rng.uniform
    rand = rng.random
    randint = rng.randint

    n_species = len(SPECIES)
    n_hotspots = len(HOTSPOTS)
    n_dates = len(DATES)

    print(f"Generating {name} ({num_rows:,} rows) -> {filepath}")
    t0 = time.time()

    buf = []
    buf_count = 0

    with open(filepath, "w", buffering=8 * 1024 * 1024) as f:
        f.write("id,species,observed_on,quality_grade,latitude,longitude,count\n")

        for row_id in range(1, num_rows + 1):
            # Location: 70% hotspot cluster, 30% uniform across bounding box.
            if rand() < 0.70:
                h = HOTSPOTS[randint(0, n_hotspots - 1)]
                lat = gauss(h[0], h[2])
                lon = gauss(h[1], h[2])
                if lat < LAT_MIN: lat = LAT_MIN
                elif lat > LAT_MAX: lat = LAT_MAX
                if lon < LON_MIN: lon = LON_MIN
                elif lon > LON_MAX: lon = LON_MAX
            else:
                lat = uniform(LAT_MIN, LAT_MAX)
                lon = uniform(LON_MIN, LON_MAX)

            # Quality grade: weighted random (60 / 25 / 15).
            qr = randint(1, 100)
            if qr <= 60:
                qg = "research"
            elif qr <= 85:
                qg = "casual"
            else:
                qg = "needs_id"

            buf.append(
                f"{row_id},{SPECIES[randint(0, n_species - 1)]},{DATES[randint(0, n_dates - 1)]},{qg},{lat:.4f},{lon:.4f},{pick_count(rand())}\n"
            )
            buf_count += 1

            if buf_count >= CHUNK_SIZE:
                f.write("".join(buf))
                buf.clear()
                buf_count = 0

            if row_id % REPORT_EVERY == 0:
                elapsed = time.time() - t0
                rate = row_id / elapsed
                eta = (num_rows - row_id) / rate
                pct = row_id / num_rows * 100
                print(f"  {row_id:>13,} / {num_rows:,}  ({pct:5.1f}%)  {rate:,.0f} rows/s  ETA {eta:.0f}s")

        if buf:
            f.write("".join(buf))

    elapsed = time.time() - t0
    size_mb = os.path.getsize(filepath) / (1024 * 1024)
    rate = num_rows / elapsed if elapsed else 0
    print(f"  Done: {name} — {size_mb:,.1f} MB in {elapsed:.1f}s ({rate:,.0f} rows/s)\n")


PRESETS = {
    "bench": [
        ("bench_10k.csv", 10_000),
        ("bench_100k.csv", 100_000),
        ("bench_1m.csv", 1_000_000),
    ],
    "250m": [
        ("bench_250m.csv", 250_000_000),
    ],
}


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--preset", choices=sorted(PRESETS),
                        help="Generate a standard set of files into testdata/.")
    parser.add_argument("--out", help="Output CSV path (single-file mode).")
    parser.add_argument("--rows", type=int, help="Number of rows (single-file mode).")
    parser.add_argument("--seed", type=int, default=42, help="RNG seed [default: 42].")
    args = parser.parse_args()

    if args.preset:
        for filename, rows in PRESETS[args.preset]:
            generate(os.path.join(SCRIPT_DIR, filename), rows, seed=args.seed)
        print("All benchmark files generated.")
    elif args.out and args.rows:
        generate(args.out, args.rows, seed=args.seed)
    else:
        parser.error("provide --preset, or both --out and --rows")


if __name__ == "__main__":
    main()
