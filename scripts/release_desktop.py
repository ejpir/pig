#!/usr/bin/env python3
"""Validate a complete four-target release, plus the Android app, and emit a SHA-256 manifest."""

import argparse
import hashlib
import json
import re
import runpy
import struct
import sys
import tarfile
import tomllib
import zipfile
from pathlib import Path

from package_desktop import ROOT, TARGETS, archive_name
from package_remote import asset_name

# Built by crates/pi_android/scripts/build_apk.py in CI's Android job.
ANDROID_APK = "pi-android-arm64.apk"
BUN_RUNTIME_ASSETS = {
    "pi-bun-android-arm64-runtime.tar.gz",
    "pi-bun-android-arm64-relink.tar.gz",
}
ANDROID_LICENSES = {
    "NotoEmoji-OFL.txt": ROOT / "crates/gpui_android/assets/fonts/OFL.txt",
    "IBM-PLEX-OFL.txt": ROOT / "licenses/IBM-PLEX-OFL.txt",
    "COMMIT-MONO-OFL.txt": ROOT / "licenses/COMMIT-MONO-OFL.txt",
    "QUICKJS-NG-MIT.txt": ROOT / "licenses/QUICKJS-NG-MIT.txt",
    "RQUICKJS-MIT.txt": ROOT / "licenses/RQUICKJS-MIT.txt",
    **{
        name: ROOT / "licenses" / name
        for name in (
            "ROXMLTREE-MIT.txt",
            "SVGTYPES-MIT.txt",
            "KURBO-MIT.txt",
            "LYON-MIT.txt",
        )
    },
}
TAG = re.compile(r"v[0-9]+\.[0-9]+\.[0-9]+(?:-[A-Za-z0-9]+(?:[.-][A-Za-z0-9]+)*)?")


def validate_tag(tag: str) -> None:
    """Reject branch names and unsafe shell/path input at the release boundary."""
    if TAG.fullmatch(tag) is None:
        raise ValueError(
            f"Expected a version tag such as v0.1.0 or v0.1.0-rc.1: {tag!r}"
        )


def validate_release_version(tag: str, version: str) -> None:
    """The desktop downloads helpers by its compiled version, so the release tag must match."""
    validate_tag(tag)
    if tag != f"v{version}":
        raise ValueError(
            f"Release tag {tag!r} does not match workspace version {version!r}"
        )


def validate_android_notices(apk: Path) -> None:
    """Do not publish embedded fonts or script runtimes without their complete reviewed notices."""
    try:
        with zipfile.ZipFile(apk) as archive:
            for name, source in ANDROID_LICENSES.items():
                if archive.read(f"assets/licenses/{name}") != source.read_bytes():
                    raise ValueError(
                        f"APK has an incomplete or changed embedded notice: {name}"
                    )
    except (zipfile.BadZipFile, KeyError) as error:
        raise ValueError(
            f"APK is invalid or missing embedded notices: {apk}"
        ) from error


def release_android_manifest() -> bytes:
    """Compare actual binary XML, not a self-asserted package/debug metadata field."""
    app = runpy.run_path(str(ROOT / "crates/pi_android/scripts/build_apk.py"))["PI"]
    builder = sys.modules["build_apk"]
    return builder.encode_xml(builder.manifest(app, debuggable=False))


def validate_local_apk(apk: Path) -> dict:
    validate_android_notices(apk)
    try:
        with zipfile.ZipFile(apk) as archive:
            if len(archive.namelist()) != len(set(archive.namelist())):
                raise ValueError("Duplicate APK entries are not publishable")
            if archive.read("AndroidManifest.xml") != release_android_manifest():
                raise ValueError(
                    "Release APK package/version/debug configuration does not match"
                )
            metadata = json.loads(archive.read("assets/local-runtime.json"))
            version = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"][
                "package"
            ]["version"]
            if any(
                metadata.get(key) != value
                for key, value in {
                    "version": version,
                    "status": "experimental",
                    "engine": "bun",
                    "bun": "1.4.3",
                    "sdk": "1.3.0",
                    "abi": "arm64-v8a",
                }.items()
            ):
                raise ValueError("APK Local runtime identity/version mismatch")
            bundle = archive.read("assets/bun-local.mjs")
            if hashlib.sha256(bundle).hexdigest() != metadata["bundleSha256"]:
                raise ValueError("APK SDK bundle checksum mismatch")
            if bundle not in archive.read("lib/arm64-v8a/libpi_android.so"):
                raise ValueError(
                    "APK native application does not embed its declared SDK bundle"
                )
            with archive.open("lib/arm64-v8a/libbunembeddingpoc.so") as library:
                header = library.read(64)
            if (
                len(header) < 64
                or header[:6] != b"\x7fELF\x02\x01"
                or struct.unpack_from("<HH", header, 16) != (3, 183)
            ):
                raise ValueError(
                    "APK Bun runtime must be an ARM64 ELF64 shared library"
                )
            with archive.open("lib/arm64-v8a/libbunembeddingpoc.so") as library:
                if (
                    hashlib.file_digest(library, "sha256").hexdigest()
                    != metadata["librarySha256"]
                ):
                    raise ValueError("APK Bun library checksum mismatch")
            for name in [
                "LICENSE.bun.md",
                "BUN-LINKED-NOTICES.txt",
                "BUN-DURABLE-NOTICES.txt",
                "LOCAL-AUTH-NPM-NOTICES.txt",
            ]:
                if not archive.read("assets/licenses/" + name).strip():
                    raise ValueError("Missing Local runtime notice: " + name)
            return metadata
    except (zipfile.BadZipFile, KeyError, json.JSONDecodeError) as error:
        raise ValueError(
            "Release APK is missing its bundled Local runtime/SDK/notices"
        ) from error


def validate_runtime_archives(directory: Path, metadata: dict) -> None:
    """The downloadable library must match the APK; static JSC relink inputs ship too."""
    try:
        with tarfile.open(
            directory / "pi-bun-android-arm64-runtime.tar.gz", "r:gz"
        ) as archive:
            libraries = [
                member
                for member in archive.getmembers()
                if Path(member.name).name == "libbunembeddingpoc.so"
            ]
            if len(libraries) != 1 or not libraries[0].isfile():
                raise ValueError(
                    "Runtime archive must contain one real Bun shared library"
                )
            with archive.extractfile(libraries[0]) as source:
                if (
                    hashlib.file_digest(source, "sha256").hexdigest()
                    != metadata["librarySha256"]
                ):
                    raise ValueError(
                        "Downloaded runtime does not match the bundled APK"
                    )
        with tarfile.open(
            directory / "pi-bun-android-arm64-relink.tar.gz", "r:gz"
        ) as archive:
            members = archive.getmembers()
            if not any(
                member.isfile() and Path(member.name).suffix in {".o", ".a", ".rlib"}
                for member in members
            ):
                raise ValueError("Missing LGPL relink object inputs")
            if not any(
                member.isfile()
                and Path(member.name).name.lower() in {"relink.py", "relink.sh"}
                for member in members
            ):
                raise ValueError("Missing executable relink instructions")
    except (tarfile.TarError, KeyError) as error:
        raise ValueError("Invalid Bun runtime/relink archives") from error


def manifest(directory: Path) -> Path:
    """Fail closed on missing/extra files rather than publishing a partial matrix."""
    expected = (
        {archive_name(target) for target in TARGETS}
        | {asset_name(target) for target in TARGETS}
        | {ANDROID_APK}
        | BUN_RUNTIME_ASSETS
    )
    entries = {file.name for file in directory.iterdir()}
    if entries != expected:
        raise ValueError(
            f"Release matrix mismatch: missing={sorted(expected - entries)}, extra={sorted(entries - expected)}"
        )
    lines = []
    for name in sorted(expected):
        file = directory / name
        if file.is_symlink() or not file.is_file() or file.stat().st_size == 0:
            raise ValueError(f"Not a nonempty release archive: {name}")
        with file.open("rb") as source:
            digest = hashlib.file_digest(source, "sha256").hexdigest()
        lines.append(f"{digest}  {name}\n")
    metadata = validate_local_apk(directory / ANDROID_APK)
    validate_runtime_archives(directory, metadata)
    destination = directory / "SHA256SUMS"
    destination.write_text("".join(lines), encoding="ascii")
    return destination


def main() -> None:
    """A tag-only check can run before costly native builds."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--directory", type=Path)
    parser.add_argument("--android-apk", type=Path)
    args = parser.parse_args()
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"][
        "version"
    ]
    validate_release_version(args.tag, version)
    if args.android_apk:
        validate_local_apk(args.android_apk)
        print("Verified bundled experimental Local APK:", args.android_apk)
    if args.directory:
        print(manifest(args.directory))


if __name__ == "__main__":
    main()
