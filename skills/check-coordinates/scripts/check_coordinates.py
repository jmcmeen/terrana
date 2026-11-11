"""Check a point dataset's coordinates before it's used; flag rows that need a look.

Counts rows with no point (a coordinate blank, not a number, NaN or infinite),
coordinates out of range (latitude -90..90, longitude -180..180), points at exactly (0, 0), and, given
an expected ``--bbox``, valid points outside it. Writes ``report.json`` (every flagged
row id) to ``runs/check-coordinates/<timestamp>/``. The check fails when the flagged
share of rows is above ``--max-flagged-fraction``, or when there are no rows: an
objective check, not a judgement.

Usage (with a data file, or against TERRANA_URL's server without one):
    python scripts/check_coordinates.py observations.csv --bbox 35,-84,37,-81

Prints a JSON summary. Exit 0: the check passed; 2: it failed; 1: terrana failed or
the arguments were wrong (``{"error": ...}``).
"""

from __future__ import annotations

import argparse
import json
import math
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from _terrana import Server, TerranaError, run_dir, server, write_json  # noqa: E402

SKILL = "check-coordinates"


class Box:
    """An expected extent. ``min_lon > max_lon`` means it crosses the antimeridian."""

    def __init__(self, text: str):
        try:
            self.min_lat, self.min_lon, self.max_lat, self.max_lon = (
                float(v) for v in text.split(",")
            )
        except ValueError:
            raise ValueError(
                f"--bbox needs min_lat,min_lon,max_lat,max_lon, not {text!r}"
            ) from None
        if not (-90 <= self.min_lat <= self.max_lat <= 90):
            raise ValueError("--bbox latitudes must be -90..90, min first")
        if not (-180 <= self.min_lon <= 180 and -180 <= self.max_lon <= 180):
            raise ValueError("--bbox longitudes must be -180..180")
        self.text = text

    def __contains__(self, point: tuple[float, float]) -> bool:
        lat, lon = point
        if not self.min_lat <= lat <= self.max_lat:
            return False
        if self.min_lon <= self.max_lon:
            return self.min_lon <= lon <= self.max_lon
        return lon >= self.min_lon or lon <= self.max_lon


def _box(text: str) -> Box:
    try:
        return Box(text)
    except ValueError as e:
        raise argparse.ArgumentTypeError(str(e)) from None


def _valid(lat, lon) -> bool:
    """A finite latitude and longitude in range."""
    return (
        isinstance(lat, (int, float))
        and isinstance(lon, (int, float))
        and math.isfinite(lat)
        and math.isfinite(lon)
        and -90 <= lat <= 90
        and -180 <= lon <= 180
    )


def check(srv: Server, box: Box | None, max_fraction: float, examples: int) -> dict:
    """Run the checks against ``srv`` in one pass over its points; returns the report."""
    schema = srv.get("/schema")
    lat, lon = schema["lat_column"], schema["lon_column"]
    rows = schema["row_count"]
    # Fetch only what the checks need. Older terrana's `select` takes only plain names,
    # and none takes a name with a comma, so for any other name fetch whole rows.
    plain = all(re.fullmatch(r"\w+", c) for c in (lat, lon))
    params = [("select", f"rowid,{lat},{lon}")] if plain else []

    flags: dict[str, list[int]] = {"missing_point": [], "out_of_range": [], "null_island": []}
    if box:
        flags["outside_bbox"] = []
    returned = 0
    for row in srv.rows(params):
        returned += 1
        rowid, y, x = row["rowid"], row.get(lat), row.get(lon)
        # terrana sends a blank, unparseable, NaN or infinite coordinate as null.
        if y is None or x is None:
            flags["missing_point"].append(rowid)
            continue
        if not _valid(y, x):
            flags["out_of_range"].append(rowid)
            continue
        if y == 0 and x == 0:
            flags["null_island"].append(rowid)
        if box and (y, x) not in box:
            flags["outside_bbox"].append(rowid)

    # Older terrana leaves rows without a point out of every query: count them too.
    unreturned = rows - returned
    missing = unreturned + len(flags["missing_point"])
    flagged = unreturned + len(set().union(*flags.values()))
    fraction = flagged / rows if rows else 0.0
    passed = rows > 0 and fraction <= max_fraction
    return {
        "source": schema.get("source"),
        "lat_column": lat,
        "lon_column": lon,
        "rows": rows,
        "with_point": rows - missing,
        "counts": {**{k: len(v) for k, v in flags.items()}, "missing_point": missing},
        "row_ids": flags,
        "examples": {k: v[:examples] for k, v in flags.items() if v},
        "flagged_rows": flagged,
        "flagged_fraction": round(fraction, 6),
        "max_flagged_fraction": max_fraction,
        "expected_bbox": box.text if box else None,
        "passed": passed,
        **({} if rows else {"reason": "the dataset has no rows"}),
    }


class _Parser(argparse.ArgumentParser):
    """Bad arguments print ``{"error": ...}`` and exit 1, not argparse's 2, which here
    means a check failed."""

    def error(self, message: str):
        print(json.dumps({"error": f"{self.prog}: {message}"}))
        sys.exit(1)


def main(argv: list[str] | None = None) -> int:
    parser = _Parser(description=__doc__.splitlines()[0])
    parser.add_argument("file", nargs="?", help="Data file (omit when TERRANA_URL is set)")
    parser.add_argument("--lat", help="Latitude column (default: terrana detects it)")
    parser.add_argument("--lon", help="Longitude column (default: terrana detects it)")
    parser.add_argument(
        "--bbox",
        type=_box,
        help="Expected extent: min_lat,min_lon,max_lat,max_lon "
        "(min_lon > max_lon crosses the antimeridian)",
    )
    parser.add_argument(
        "--max-flagged-fraction",
        type=float,
        default=0.0,
        help="Largest share of flagged rows that still passes (default 0: any flag fails)",
    )
    parser.add_argument("--examples", type=int, default=5, help="Row ids shown per flag")
    parser.add_argument("--root", default=".", help="Where runs/ goes (default: here)")
    argv = list(sys.argv[1:] if argv is None else argv)
    # A southern box starts with "-" ("--bbox -25,170,..."), which argparse would read
    # as an option; join it to its flag.
    for i, arg in enumerate(argv[:-1]):
        if arg == "--bbox":
            argv[i : i + 2] = [f"--bbox={argv[i + 1]}"]
            break
    args = parser.parse_args(argv)

    serve_args = [a for opt, v in (("--lat", args.lat), ("--lon", args.lon)) if v for a in (opt, v)]
    try:
        with server(args.file, *serve_args) as srv:
            report = check(srv, args.bbox, args.max_flagged_fraction, args.examples)
    except (TerranaError, KeyError) as e:
        message = f"unexpected answer from terrana, missing {e}" if isinstance(e, KeyError) else e
        print(json.dumps({"error": str(message)}))
        return 1
    out = run_dir(SKILL, args.root)
    write_json(out / "report.json", report)
    summary = {k: v for k, v in report.items() if k != "row_ids"}
    print(json.dumps({"run_dir": str(out.relative_to(args.root)), **summary}, indent=2))
    return 0 if report["passed"] else 2


if __name__ == "__main__":
    sys.exit(main())
