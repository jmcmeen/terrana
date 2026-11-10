<!-- Thanks for contributing to Terrana! Please fill out the checklist below. -->

## Summary

<!-- What does this PR change, and why? -->

## Related issue

<!-- e.g. Closes #123 -->

## Checklist

- [ ] `cargo fmt --all` — code is formatted
- [ ] `cargo clippy --all-targets -- -D warnings` — no lint warnings
- [ ] `cargo test` passes (it includes the integration tests; the first run needs network access to cache DuckDB's spatial extension)
- [ ] Added/updated tests for the change
- [ ] Updated `CHANGELOG.md` (Unreleased section)
- [ ] Updated README / docs for any user-facing change
- [ ] Geometry math uses geodesic algorithms from the `geo` crate (no planar math)
