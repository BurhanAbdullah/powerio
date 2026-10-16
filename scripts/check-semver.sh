#!/usr/bin/env bash
# Public Rust API compatibility: compare every publishable crate against the
# newest stable release tag at or below the workspace version with
# cargo-semver-checks.
#
# A release line takes compatible fixes, performance work, and additive change;
# a break in the public Rust API goes into the next minor release
# (docs/src/scope-0.11.md). cargo-semver-checks reads the release type off the
# version difference, so a branch that keeps the workspace version must keep
# the API, and a branch that bumps it to the next minor may break it. Taking
# the newest tag at or below the workspace version keeps a maintenance branch
# on its own line after a later minor is tagged.
#
# The published v0.11.x baseline lockfiles reference zip 8.6.0, which has been
# yanked. cargo-semver-checks creates temporary consumer manifests and runs
# cargo update; those consumers cannot use the baseline lockfile to resolve a
# yanked release. Build the baseline from a temporary source snapshot and
# constrain only that snapshot to the last non-yanked compatible 8.x release.
# This keeps the API comparison enabled; it does not skip any crate or lint.
#
# The tags must be present: a shallow clone fails here rather than comparing
# against nothing. Extra arguments go to cargo-semver-checks.
set -euo pipefail
cd "$(dirname "$0")/.."

if ! cargo semver-checks --version >/dev/null 2>&1; then
    echo "cargo-semver-checks is not installed; run: cargo install cargo-semver-checks --locked" >&2
    exit 2
fi

version=$(grep -oE '^version = "[0-9]+\.[0-9]+\.[0-9]+"' Cargo.toml | head -1 | grep -oE '[0-9]+\.[0-9]+\.[0-9]+')
baseline=
for tag in $(git tag --list 'v*' | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' | sort -V); do
    if [ "$(printf '%s\n%s\n' "${tag#v}" "$version" | sort -V | tail -n1)" = "$version" ]; then
        baseline=$tag
    fi
done
if [ -z "$baseline" ]; then
    echo "no stable release tag at or below $version; fetch the tags (git fetch --tags)" >&2
    exit 1
fi

baseline_root=$(mktemp -d "${TMPDIR:-/tmp}/powerio-semver-baseline.XXXXXX")
current_root=$(mktemp -d "${TMPDIR:-/tmp}/powerio-semver-current.XXXXXX")
trap 'rm -rf "$baseline_root" "$current_root"' EXIT

# Use source snapshots for both sides, since cargo-semver-checks builds each
# package as a standalone consumer and runs cargo update. The yanked zip 8.6.0
# breaks that consumer resolution even when the repository Cargo.lock works.
git archive "$baseline" | tar -x -C "$baseline_root"
git archive HEAD | tar -x -C "$current_root"

# Narrow only the temporary validation snapshots to zip 8.5.1. The library API
# being compared is unchanged; this avoids a resolver outage from a yanked
# dependency while keeping every publishable crate and SemVer lint enabled.
python3 - "$baseline_root/Cargo.toml" "$current_root/Cargo.toml" <<'PY'
from pathlib import Path
import sys

for filename in sys.argv[1:]:
    path = Path(filename)
    manifest = path.read_text()
    old = 'zip = { version = "8.6",'
    new = 'zip = { version = "8.5.1",'
    if old not in manifest:
        raise SystemExit(f"expected workspace zip dependency not found in {path}")
    path.write_text(manifest.replace(old, new, 1))
PY

echo "comparing publishable crates at $version against $baseline using isolated resolver-safe snapshots"
# powerio-py and powerio-capi are publish = false; exclude them, not any
# publishable package or SemVer lint.
cd "$current_root"
cargo semver-checks check-release --workspace \
    --exclude powerio-py --exclude powerio-capi \
    --baseline-root "$baseline_root" "$@"
