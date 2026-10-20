#!/usr/bin/env python3
"""Prepare temporary source trees for cargo-semver-checks compatibility."""

from __future__ import annotations

import pathlib
import sys
import tempfile

OLD_ZIP = 'zip = { version = "8.6", default-features = false, features = ["deflate-flate2-zlib-rs"] }'
NEW_ZIP = 'zip = { version = "9.0.3", default-features = false, features = ["deflate-flate2-zlib-rs"] }'

REPLACEMENTS = [
    (
        "let raw_name = file.name().to_string();",
        """let raw_name = file
            .name()
            .map_err(|error| {
                format_error(format!(
                    "cannot decode entry {index} name in {archive_name}: {error}"
                ))
            })?
            .into_owned();""",
    ),
    (
        """\"archive {archive_name} is refused as malformed: entry {} lies outside the archive's entry data\",
                    entry.name()""",
        """\"archive {archive_name} is refused as malformed: entry {index} lies outside the archive's entry data\"""",
    ),
    (
        "let name = |index| archive.name_for_index(index).unwrap_or_default();",
        """let name = |index| {
            archive
                .name_for_index(index)
                .and_then(std::result::Result::ok)
                .map(std::borrow::Cow::into_owned)
                .unwrap_or_default()
        };""",
    ),
]


def patch_root(root: pathlib.Path) -> None:
    """Patch only known ZIP dependency/API compatibility anchors in a temp tree."""
    manifest = root / "Cargo.toml"
    text = manifest.read_text()
    if OLD_ZIP in text:
        if text.count(OLD_ZIP) != 1:
            raise SystemExit(f"expected at most one ZIP 8.6 dependency in {root}")
        manifest.write_text(text.replace(OLD_ZIP, NEW_ZIP))
    elif 'zip = { version = "9.' not in text:
        raise SystemExit(f"expected a supported ZIP 8.6 or 9.x dependency in {root}")

    source = root / "powerio-tx/src/format/cgmes/mod.rs"
    text = source.read_text()
    for old_anchor, new_anchor in REPLACEMENTS:
        if old_anchor not in text:
            # ZIP 9-compatible source may already contain this API adjustment.
            continue
        if text.count(old_anchor) != 1:
            raise SystemExit(
                f"expected exactly one ZIP API compatibility anchor in {root}: {old_anchor!r}"
            )
        text = text.replace(old_anchor, new_anchor)
    source.write_text(text)


def self_test() -> None:
    """Regression: an 8.6 baseline and already-upgraded 9.x current tree coexist."""
    with tempfile.TemporaryDirectory() as temp:
        base = pathlib.Path(temp)
        roots = [base / "baseline", base / "current"]
        for root, version in zip(roots, ("8.6", "9.0.5"), strict=True):
            (root / "powerio-tx/src/format/cgmes").mkdir(parents=True)
            (root / "Cargo.toml").write_text(
                f'zip = {{ version = "{version}", default-features = false, '
                'features = ["deflate-flate2-zlib-rs"] } }\n'
            )
            (root / "powerio-tx/src/format/cgmes/mod.rs").write_text("// fixture\n")

        patch_root(roots[0])
        patch_root(roots[1])
        baseline = (roots[0] / "Cargo.toml").read_text()
        current = (roots[1] / "Cargo.toml").read_text()
        assert NEW_ZIP in baseline, "ZIP 8.6 baseline must be rewritten"
        assert 'version = "9.0.5"' in current, "ZIP 9 current tree must be preserved"
        assert NEW_ZIP not in current, "current ZIP 9 version must not be forced to 9.0.3"


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        self_test()
    elif len(sys.argv) > 1:
        for argument in sys.argv[1:]:
            patch_root(pathlib.Path(argument))
    else:
        raise SystemExit("usage: semver_zip_compat.py --self-test | ROOT [ROOT ...]")
