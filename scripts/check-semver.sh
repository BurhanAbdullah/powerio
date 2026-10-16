#!/usr/bin/env bash
# Public Rust API compatibility: compare every publishable crate against the
# newest stable release tag at or below the workspace version with
# cargo-semver-checks.
#
# Use the tagged release's own dependency versions. In particular, do not
# rewrite the zip dependency in temporary snapshots: newer zip releases can
# change APIs and make rustdoc fail before the SemVer comparison runs.
#
# cargo-semver-checks creates temporary consumer manifests and runs cargo
# update. If a historical lockfile references a yanked release, recover that
# exact crate archive from the repository's committed Cargo.lock and local
# Cargo cache when available, rather than changing the dependency requirement.
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

echo "comparing publishable crates at $version against $baseline"
# powerio-py and powerio-capi are publish = false; exclude only those crates.
cargo semver-checks check-release --workspace \
    --exclude powerio-py --exclude powerio-capi \
    --baseline-rev "$baseline" "$@"
