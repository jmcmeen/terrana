#!/usr/bin/env python3
"""Generate a 250M row benchmark CSV for Terrana.

Optimized for speed: pre-computes lookup tables, writes raw strings
in large buffered chunks, avoids csv module overhead.

Usage: python3 testdata/generate_250m.py
Output: testdata/bench_250m.csv (~14-15 GB)
"""

import os
import random
import time

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
OUTPUT = os.path.join(SCRIPT_DIR, "bench_250m.csv")
NUM_ROWS = 250_000_000
CHUNK_SIZE = 100_000  # rows per write buffer
REPORT_EVERY = 1_000_000

# --- Pre-build species list ---
SPECIES = [
    "Eastern Bluebird", "Carolina Chickadee", "Red-tailed Hawk",
    "Pileated Woodpecker", "Scarlet Tanager", "Wood Thrush",
    "Cerulean Warbler", "Peregrine Falcon", "Barred Owl",
    "Ruffed Grouse", "Wild Turkey", "Dark-eyed Junco",
    "White-tailed Deer", "Black Bear", "Eastern Chipmunk",
    "Gray Squirrel", "Red Fox", "Coyote", "Bobcat",
    "Northern Flying Squirrel", "Elk",
    "Eastern Box Turtle", "Timber Rattlesnake", "Eastern Fence Lizard",
    "Northern Copperhead",
    "Eastern Hellbender", "Red-spotted Newt", "Jordan's Salamander",
    "Spring Peeper",
    "Flame Azalea", "Fraser Fir",
]

QUALITY_GRADES = ["research", "casual", "needs_id"]
QUALITY_WEIGHTS = [60, 25, 15]

HOTSPOTS = [
    (35.61, -83.53, 0.15),
    (35.63, -83.20, 0.12),
    (35.70, -82.50, 0.18),
    (36.05, -81.80, 0.14),
    (36.60, -81.40, 0.12),
    (35.30, -82.80, 0.13),
    (35.20, -83.55, 0.11),
    (35.10, -84.10, 0.10),
    (36.10, -82.40, 0.12),
    (35.78, -83.00, 0.09),
]

LAT_MIN, LAT_MAX = 34.5, 37.0
LON_MIN, LON_MAX = -84.5, -81.0

# Pre-compute all dates as strings (2023-01-01 to 2024-12-31 = 731 days)
from datetime import date, timedelta
_date_start = date(2023, 1, 1)
_date_end = date(2024, 12, 31)
_num_days = (_date_end - _date_start).days
DATES = [(_date_start + timedelta(days=d)).isoformat() for d in range(_num_days + 1)]

# Pre-compute count distribution (1/x weighted, values 1-20)
_count_weights = [1.0 / x for x in range(1, 21)]
_count_total = sum(_count_weights)
_count_cum = []
_c = 0.0
for w in _count_weights:
    _c += w / _count_total
    _count_cum.append(_c)

def pick_count(r):
    """r is a random float [0,1)."""
    for i, threshold in enumerate(_count_cum):
        if r <= threshold:
            return i + 1
    return 20


def main():
    rng = random.Random(42)
    gauss = rng.gauss
    uniform = rng.uniform
    rand = rng.random
    randint = rng.randint
    choice = rng.choice

    n_species = len(SPECIES)
    n_hotspots = len(HOTSPOTS)
    n_dates = len(DATES)

    print(f"Generating {NUM_ROWS:,} rows -> {OUTPUT}")
    print(f"Estimated size: ~{NUM_ROWS * 60 / 1e9:.1f} GB")
    t0 = time.time()

    buf = []
    buf_count = 0
    rows_written = 0

    with open(OUTPUT, "w", buffering=8 * 1024 * 1024) as f:
        f.write("id,species,observed_on,quality_grade,latitude,longitude,count\n")

        for row_id in range(1, NUM_ROWS + 1):
            # Location: 70% hotspot, 30% uniform
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

            # Quality grade: weighted random
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
                eta = (NUM_ROWS - row_id) / rate
                pct = row_id / NUM_ROWS * 100
                print(f"  {row_id:>13,} / {NUM_ROWS:,}  ({pct:5.1f}%)  {rate:,.0f} rows/s  ETA {eta:.0f}s")

        # Flush remaining
        if buf:
            f.write("".join(buf))

    elapsed = time.time() - t0
    size_gb = os.path.getsize(OUTPUT) / (1024 ** 3)
    rate = NUM_ROWS / elapsed
    print(f"\nDone: {size_gb:.2f} GB in {elapsed:.1f}s ({rate:,.0f} rows/s)")


if __name__ == "__main__":
    main()
