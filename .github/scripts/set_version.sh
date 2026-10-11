#!/usr/bin/env bash
# Set terrana's version (X.Y.Z) everywhere it's recorded. Run by release.yml in
# the release commit; crates.io publishes the Cargo.toml version, and publish.yml
# refuses to publish when it doesn't match the tag.
set -euo pipefail
version="${1:?usage: set_version.sh X.Y.Z}"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "not a version: $version" >&2; exit 1; }

# The first `version =` line is [package]'s; dependency versions come later.
for f in Cargo.toml python/Cargo.toml; do
  perl -0pi -e "s/^version = \"[^\"]*\"/version = \"$version\"/m" "$f"
done
# Our own packages in the lockfiles (dependencies are left alone).
for f in Cargo.lock python/Cargo.lock; do
  perl -0pi -e "s/(\nname = \"terrana(?:-py)?\"\nversion = )\"[^\"]*\"/\$1\"$version\"/g" "$f"
done
perl -pi -e "s/^version: .*/version: $version/; s/^date-released: .*/date-released: $(date -u +%Y-%m-%d)/" CITATION.cff
