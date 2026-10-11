# Contributing to Terrana

Thanks for your interest in contributing! Terrana is a zero-config spatial API
server written in Rust, and contributions of all kinds are welcome — bug reports,
documentation, new file formats, geometry operations, and performance work.

## Ways to contribute

- **Bug reports** — Open an issue describing the problem, the file format you used,
  and the exact query that triggered it. A minimal sample file helps enormously.
- **New file formats** — Terrana ingests through DuckDB; adding a format usually
  means a small addition to [`src/db/loader.rs`](src/db/loader.rs).
- **Geometry operations** — The geodesic math lives in pure functions under
  [`src/geometry/`](src/geometry/) (`area.rs`, `buffer.rs`, `hull.rs`, `measure.rs`, …);
  the `POST /geometry/*` handlers in
  [`src/handlers/geometry.rs`](src/handlers/geometry.rs) are thin glue over them. All
  spatial math **must** use geodesic algorithms from the `geo` crate — never
  planar/Cartesian (see [Geodesic rules](#geodesic-rules) below).
- **Python bindings** — The PyO3 bindings live in [`python/`](python/) and re-export
  the same engine; new Python surface goes in [`python/src/lib.rs`](python/src/lib.rs)
  with a test in [`python/tests/`](python/tests/).
- **Performance** — Benchmark with `./testdata/bench.sh` (or `make bench`), profile with `cargo flamegraph`,
  and open a PR with before/after numbers.
- **Documentation** — Improvements to the README, examples, or inline doc comments
  are always appreciated.

## Development setup

You'll need a Rust toolchain (stable). If you don't have one, see
[Installing Rust](README.md#installing-rust) in the README.

```bash
git clone https://github.com/jmcmeen/terrana.git
cd terrana
cargo build
cargo run -- serve testdata/observations.csv
```

No system DuckDB or PostGIS is required — DuckDB is bundled. On first run, DuckDB
downloads its `spatial` extension from the network and caches it locally.

## Before you open a pull request

Run the same checks CI runs, and make sure they all pass:

```bash
cargo fmt --all                       # format
cargo clippy --all-targets -- -D warnings   # lint (warnings are errors)
cargo test                            # unit tests (offline)
cargo test -- --include-ignored       # + integration tests (needs network)
```

Or use the [`Makefile`](Makefile) shortcuts (`make help` lists them all):

```bash
make ci         # fmt-check + lint + unit tests (the offline gate)
make test-all   # unit + integration tests (needs network)
make run        # run the server against testdata/observations.csv
```

The integration tests in [`tests/api.rs`](tests/api.rs) spawn the real binary and
hit the HTTP endpoints. They are `#[ignore]`d by default because starting the server
requires the DuckDB `spatial` extension to be available; run them with
`--include-ignored` in an environment with network access.

For changes to the **Python bindings**, build and test them with
[uv](https://docs.astral.sh/uv/):

```bash
cd python
uv venv --python 3.13 && source .venv/bin/activate
uv pip install maturin pytest
maturin develop              # build the extension into the venv
uv run pytest tests/ -v      # library + server-mode tests
```

## Pull request guidelines

- Keep PRs focused — one logical change per PR.
- Add or update tests for any behavior change.
- Update [`CHANGELOG.md`](CHANGELOG.md) under the `Unreleased` section.
- Update the README and inline docs when you change user-facing behavior.
- Write commit messages in the imperative mood ("Add buffer endpoint", not "Added").

## Releasing & versioning

Releases are automatic. The Rust crate, the binaries, and the Python wheel ship
from a **single `vX.Y.Z` tag** and **share one version**:

- A merge to `main` that passes CI is released as the next **patch** version
  (merges that land while a release is running share the next one)
  ([`.github/workflows/release.yml`](.github/workflows/release.yml)). A bot commit
  on `main` moves the `[Unreleased]` changelog entries under the new version and
  sets it in `Cargo.toml`, `python/Cargo.toml`, both `Cargo.lock`s, and
  `CITATION.cff` ([`.github/scripts/set_version.sh`](.github/scripts/set_version.sh));
  that commit is tagged, and the entries become the GitHub release notes.
- [`publish.yml`](.github/workflows/publish.yml) then attaches the binaries,
  publishes to crates.io, and starts
  [`python-publish.yml`](.github/workflows/python-publish.yml) for PyPI.
- For a **minor or major** release, the maintainer runs the workflow by hand on
  `main`: `gh workflow run release.yml -f bump=minor` (or `major`). Follow
  [SemVer](https://semver.org).
- Flow: work on `dev`, open a PR into `main`; the merge to `main` is the release.

Contributors don't touch version numbers — just note user-facing changes in
[`CHANGELOG.md`](CHANGELOG.md) under `[Unreleased]`.

## Geodesic rules

These are non-negotiable for every geometry calculation:

- **Area / perimeter** → `geo::GeodesicArea::geodesic_area_unsigned()` (Karney, WGS 84).
- **Buffer ring vertices** → `geo::Destination::geodesic_destination()`.
- **Geometry-endpoint distances** → `geo::Distance` / geodesic (ellipsoidal).
- **Query-path distances** (radius, nearest) → DuckDB `ST_Distance_Sphere` (haversine)
  is acceptable.

Never use planar/Cartesian math for area, distance, or buffer calculations.

## Scope

Terrana is intentionally small. It is **not** a PostGIS replacement, a tile server,
a distributed system, or a CRS-conversion tool (WGS 84 only). Please open an issue to
discuss before working on anything that expands this scope — see "What This Project Is
NOT" in [CLAUDE.md](CLAUDE.md).

## Licensing

Terrana is dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE).
Unless you state otherwise, any contribution you submit for inclusion in the work, as
defined in the Apache-2.0 license, shall be dual-licensed as above, without any
additional terms or conditions.
