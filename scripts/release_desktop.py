#!/usr/bin/env python3
"""Validate a complete four-target release, plus the Android app, and emit a SHA-256 manifest."""
import argparse
import hashlib
import re
import tomllib
from pathlib import Path

from package_desktop import ROOT, TARGETS, archive_name
from package_remote import asset_name

# Built by crates/pi_android/scripts/build_apk.py in CI's Android job.
ANDROID_APK = "pi-android-arm64.apk"
TAG = re.compile(r"v[0-9]+\.[0-9]+\.[0-9]+(?:-[A-Za-z0-9]+(?:[.-][A-Za-z0-9]+)*)?")


def validate_tag(tag: str) -> None:
    """Reject branch names and unsafe shell/path input at the release boundary."""
    if TAG.fullmatch(tag) is None:
        raise ValueError(f"Expected a version tag such as v0.1.0 or v0.1.0-rc.1: {tag!r}")


def validate_release_version(tag: str, version: str) -> None:
    """The desktop downloads helpers by its compiled version, so the release tag must match."""
    validate_tag(tag)
    if tag != f"v{version}":
        raise ValueError(f"Release tag {tag!r} does not match workspace version {version!r}")


def manifest(directory: Path) -> Path:
    """Fail closed on missing/extra files rather than publishing a partial matrix."""
    expected = ({archive_name(target) for target in TARGETS} | {asset_name(target) for target in TARGETS}
                | {ANDROID_APK})
    entries = {file.name for file in directory.iterdir()}
    if entries != expected:
        raise ValueError(f"Release matrix mismatch: missing={sorted(expected - entries)}, extra={sorted(entries - expected)}")
    lines = []
    for name in sorted(expected):
        file = directory / name
        if file.is_symlink() or not file.is_file() or file.stat().st_size == 0:
            raise ValueError(f"Not a nonempty release archive: {name}")
        with file.open("rb") as source:
            digest = hashlib.file_digest(source, "sha256").hexdigest()
        lines.append(f"{digest}  {name}\n")
    destination = directory / "SHA256SUMS"
    destination.write_text("".join(lines), encoding="ascii")
    return destination


def main() -> None:
    """A tag-only check can run before costly native builds."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--directory", type=Path)
    args = parser.parse_args()
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    validate_release_version(args.tag, version)
    if args.directory:
        print(manifest(args.directory))


if __name__ == "__main__":
    main()
