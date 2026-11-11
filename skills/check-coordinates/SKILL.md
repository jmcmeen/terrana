---
name: check-coordinates
description: Check a point dataset's coordinates (CSV, Parquet, GeoJSON or DuckDB) before it's used, counting rows with no point, coordinates out of range, points at (0, 0) and points outside an expected area. Use when asked whether a dataset's locations can be trusted, or before mapping, joining or analysing it by location.
allowed-tools: Bash(python3 "${CLAUDE_SKILL_DIR}/scripts/check_coordinates.py" *)
---

# Check coordinates

Runs terrana on the dataset and counts the rows whose location needs a look. Also the
minimal example of a terrana skill (see `skills/README.md` in the terrana repository).

## Run

```
python3 "${CLAUDE_SKILL_DIR}/scripts/check_coordinates.py" <file> [--bbox MIN_LAT,MIN_LON,MAX_LAT,MAX_LON]
    [--max-flagged-fraction F] [--lat COLUMN --lon COLUMN]
```

`${CLAUDE_SKILL_DIR}` is this skill's folder (the one holding this file); in an agent
that doesn't fill it in, use that folder's path.

- `--bbox`: the area the data should fall in (for example a study region). Without it,
  the outside-area check is skipped. A box whose minimum longitude is larger than its
  maximum crosses the antimeridian (`-25,170,-10,-170` spans 170°E to 170°W).
- `--max-flagged-fraction`: the share of flagged rows that still passes. The default, 0,
  fails on any flagged row; use the threshold the person gives you.
- `--lat` / `--lon`: only if terrana can't detect the coordinate columns.
- With `TERRANA_URL` set (and `TERRANA_TOKEN` if the server needs one), leave out the
  file and `--lat`/`--lon`: the script checks the data that server was started on, and
  refuses a file so it can't report on the wrong one.

It prints a JSON summary: `source`, `counts` per problem (`missing_point`, a coordinate
blank, not a number, NaN or infinite; `out_of_range`; `null_island`; `outside_bbox`),
`examples` (row ids), `flagged_rows`, `flagged_fraction`, `passed`, and `run_dir`, where
`report.json` lists every flagged row id. Exit 0: passed. Exit 2: too many rows flagged, or the dataset has no rows
(`reason` says so). Exit 1: terrana failed or the arguments were wrong
(`{"error": ...}`); fix the call or report the error, don't treat it as a verdict.

## Rules

- Report the counts, the verdict and a few example row ids as they are. Don't decide
  yourself that a flagged row is fine; the threshold decides, and a person reviews.
- A point at (0, 0) is usually a missing value written as zeros, but it can be real:
  report it, don't drop it.
- Don't edit the data file. Fixes are the person's call.
