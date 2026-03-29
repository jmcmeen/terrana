#!/usr/bin/env python3
"""Generate benchmark CSV files with simulated Southern Appalachian wildlife observations."""

import csv
import math
import os
import random
import time
from datetime import date, timedelta

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))

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
QUALITY_WEIGHTS = [60, 25, 15]  # cumulative thresholds built below
_QUALITY_CUM = []
_total = 0
for w in QUALITY_WEIGHTS:
    _total += w
    _QUALITY_CUM.append(_total)

# --- Hotspot clusters (lat, lon, std_dev) ---
# Great Smoky Mountains NP (center-ish)
# Blue Ridge Parkway (several points along it)
# Pisgah National Forest
# Nantahala National Forest
# Cherokee National Forest
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

# Date range
DATE_START = date(2023, 1, 1)
DATE_DAYS = (date(2024, 12, 31) - DATE_START).days  # inclusive

# Count weights: favor low values (1-5 heavy, tails off)
# We'll use a simple approach: pick from 1-20 with P proportional to 1/x
_COUNT_WEIGHTS = [1.0 / x for x in range(1, 21)]
_COUNT_TOTAL = sum(_COUNT_WEIGHTS)
_COUNT_CUM = []
_c = 0.0
for w in _COUNT_WEIGHTS:
    _c += w / _COUNT_TOTAL
    _COUNT_CUM.append(_c)


def pick_quality(rng):
    r = rng.randint(1, 100)
    for i, threshold in enumerate(_QUALITY_CUM):
        if r <= threshold:
            return QUALITY_GRADES[i]
    return QUALITY_GRADES[-1]


def pick_count(rng):
    r = rng.random()
    for i, threshold in enumerate(_COUNT_CUM):
        if r <= threshold:
            return i + 1
    return 20


def clamp(val, lo, hi):
    return max(lo, min(hi, val))


def pick_location(rng):
    """70% chance from a hotspot cluster, 30% uniform across bounding box."""
    if rng.random() < 0.70:
        spot = rng.choice(HOTSPOTS)
        lat = rng.gauss(spot[0], spot[2])
        lon = rng.gauss(spot[1], spot[2])
        lat = clamp(lat, LAT_MIN, LAT_MAX)
        lon = clamp(lon, LON_MIN, LON_MAX)
    else:
        lat = rng.uniform(LAT_MIN, LAT_MAX)
        lon = rng.uniform(LON_MIN, LON_MAX)
    return round(lat, 4), round(lon, 4)


def pick_date(rng):
    delta = rng.randint(0, DATE_DAYS)
    return (DATE_START + timedelta(days=delta)).isoformat()


def generate_csv(filepath, num_rows):
    name = os.path.basename(filepath)
    print(f"Generating {name} ({num_rows:,} rows)...")
    rng = random.Random(42)  # deterministic seed per file for reproducibility
    t0 = time.time()
    with open(filepath, "w", newline="") as f:
        writer = csv.writer(f)
        writer.writerow(["id", "species", "observed_on", "quality_grade",
                          "latitude", "longitude", "count"])
        for i in range(1, num_rows + 1):
            lat, lon = pick_location(rng)
            writer.writerow([
                i,
                rng.choice(SPECIES),
                pick_date(rng),
                pick_quality(rng),
                lat,
                lon,
                pick_count(rng),
            ])
            if i % 100_000 == 0:
                elapsed = time.time() - t0
                print(f"  {i:>10,} rows written  ({elapsed:.1f}s)")
    elapsed = time.time() - t0
    size_mb = os.path.getsize(filepath) / (1024 * 1024)
    print(f"  Done: {name} — {size_mb:.1f} MB in {elapsed:.1f}s\n")


def main():
    files = [
        ("bench_10k.csv", 10_000),
        ("bench_100k.csv", 100_000),
        ("bench_1m.csv", 1_000_000),
    ]
    for filename, rows in files:
        generate_csv(os.path.join(SCRIPT_DIR, filename), rows)
    print("All benchmark files generated.")


if __name__ == "__main__":
    main()
