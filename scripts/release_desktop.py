#!/usr/bin/env python3
"""Validate a complete four-target release, plus the Android app, and emit a SHA-256 manifest."""

import argparse
import hashlib
import json
import os
import re
import runpy
import struct
import subprocess
import sys
import tarfile
import tomllib
import zipfile
from pathlib import Path

from package_desktop import ROOT, TARGETS, archive_name
from package_remote import asset_name

# Built by crates/pi_android/scripts/build_apk.py in CI's Android job.
ANDROID_APK = "pi-android-arm64.apk"
ANDROID_FONT = ROOT / "assets/fonts/PiTerminalSymbolsMono-Regular.ttf"
ANDROID_FONT_SHA256 = "5a0737b45bb0ec5b7ce91c35749fa9142eb4a6aef2d89df0d8a88fb6b3694202"
ANDROID_SIGNER_SHA256 = "4b1d0ec031843d23e95f0e4a2e70125241b5e80041bdb1caecceac99a9774e71"
BUN_SOURCE_SHA256 = "2366bc56d092f8535bbe5827788e20d987fbc96798de4d683e0ca78ea286ebf4"
JSC_SOURCE_SHA256 = "88eb566d10e541c392164f164170d1ac1e0c562f23b3bd490b4b856972079b15"
JSC_PREBUILT_SHA256 = "21b9ee241f1b86d5b2db0197e081345ae3d5f0ae182e96dd7d1d3dc71f73b5f1"
DEPENDENCY_SOURCE_ARCHIVES = json.loads(
    (ROOT / "experiments/bun-embedding/dependency-sources-v1.4.3.json").read_text()
)
BUN_RUNTIME_ASSETS = {
    "pi-bun-android-arm64-runtime.tar.gz",
    "pi-bun-android-arm64-relink.tar.gz",
}
ANDROID_LICENSES = {
    "NotoEmoji-OFL.txt": ROOT / "crates/gpui_android/assets/fonts/OFL.txt",
    "LICENSE": ROOT / "LICENSE",
    "THIRD_PARTY.md": ROOT / "THIRD_PARTY.md",
    **{
        name: ROOT / "licenses" / name
        for name in (
            "IBM-PLEX-OFL.txt",
            "COMMIT-MONO-OFL.txt",
            "PI-TERMINAL-SYMBOLS-NOTICES.txt",
            "RUST-DEPENDENCY-NOTICES.html",
            "RQUICKJS-MIT.txt",
            "QUICKJS-NG-MIT.txt",
            "ROXMLTREE-MIT.txt",
            "SVGTYPES-MIT.txt",
            "KURBO-MIT.txt",
            "LYON-MIT.txt",
            "ZED-APACHE-2.0",
            "ZED-GPL-3.0",
            "ZED-ICONS-ISC.txt",
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


def validate_android_signature(apk: Path) -> None:
    """Require the established update signer, not merely a structurally valid APK."""
    android_home = os.environ.get("ANDROID_HOME") or os.environ.get("ANDROID_SDK_ROOT")
    candidates = (
        sorted(Path(android_home, "build-tools").glob("*/apksigner"))
        if android_home
        else []
    )
    if not candidates:
        raise ValueError("Android apksigner is required for release publication")
    result = subprocess.run(
        [str(candidates[-1]), "verify", "--verbose", "--print-certs", str(apk)],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    if result.returncode != 0:
        raise ValueError(f"APK signature verification failed: {result.stdout.strip()}")
    digests = {
        match.lower()
        for match in re.findall(
            r"(?:Signer #\d+ certificate SHA-256 digest|"
            r"V[0-9.]+ Signer: certificate SHA-256 digest): ([0-9a-fA-F]{64})",
            result.stdout,
        )
    }
    if digests != {ANDROID_SIGNER_SHA256}:
        raise ValueError(f"Release APK signer mismatch: {sorted(digests)}")


def validate_local_apk(apk: Path) -> dict:
    validate_android_notices(apk)
    try:
        with zipfile.ZipFile(apk) as archive:
            if len(archive.namelist()) != len(set(archive.namelist())):
                raise ValueError("Duplicate APK entries are not publishable")
            runtime_notice_names = {
                "LICENSE.bun.md",
                "BUN-LINKED-NOTICES.txt",
                "BUN-DURABLE-NOTICES.txt",
                "LOCAL-AUTH-NPM-NOTICES.txt",
            }
            actual_notices = {
                Path(name).name
                for name in archive.namelist()
                if name.startswith("assets/licenses/") and not name.endswith("/")
            }
            expected_notices = set(ANDROID_LICENSES) | runtime_notice_names
            if actual_notices != expected_notices:
                raise ValueError(
                    "APK notice inventory mismatch: "
                    f"missing={sorted(expected_notices - actual_notices)}, "
                    f"extra={sorted(actual_notices - expected_notices)}"
                )
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
            expected_bundle = (ROOT / "crates/pi_local/generated/bun-local.mjs").read_bytes()
            if bundle != expected_bundle:
                raise ValueError("APK SDK bundle does not match the regenerated tagged source")
            if hashlib.sha256(bundle).hexdigest() != metadata["bundleSha256"]:
                raise ValueError("APK SDK bundle checksum mismatch")
            application = archive.read("lib/arm64-v8a/libpi_android.so")
            if bundle not in application:
                raise ValueError(
                    "APK native application does not embed its declared SDK bundle"
                )
            font = ANDROID_FONT.read_bytes()
            if hashlib.sha256(font).hexdigest() != ANDROID_FONT_SHA256:
                raise ValueError("Pinned Android terminal font checksum mismatch")
            if font not in application:
                raise ValueError("APK does not embed the pinned Android terminal font")
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
            runtime_notice_digests = {}
            for name in sorted(runtime_notice_names):
                notice = archive.read("assets/licenses/" + name)
                if not notice.strip():
                    raise ValueError("Missing Local runtime notice: " + name)
                runtime_notice_digests[name] = hashlib.sha256(notice).hexdigest()
            for name in ["BUN-DURABLE-NOTICES.txt", "LOCAL-AUTH-NPM-NOTICES.txt"]:
                source = ROOT / "crates/pi_local/generated" / name
                if archive.read("assets/licenses/" + name) != source.read_bytes():
                    raise ValueError("APK Local SDK notice does not match generated input: " + name)
            metadata["_noticeSha256"] = runtime_notice_digests
            return metadata
    except (zipfile.BadZipFile, KeyError, json.JSONDecodeError) as error:
        raise ValueError(
            "Release APK is missing its bundled Local runtime/SDK/notices"
        ) from error


def _archive_files(archive: tarfile.TarFile) -> dict[str, tarfile.TarInfo]:
    members = archive.getmembers()
    if any(
        not member.isfile()
        or Path(member.name).is_absolute()
        or ".." in Path(member.name).parts
        for member in members
    ):
        raise ValueError("Runtime archives may contain only safe regular files")
    files = {member.name: member for member in members}
    if len(files) != len(members):
        raise ValueError("Runtime archive contains duplicate members")
    return files


def _archive_bytes(archive: tarfile.TarFile, member: tarfile.TarInfo) -> bytes:
    source = archive.extractfile(member)
    if source is None:
        raise ValueError(f"Could not read runtime archive member {member.name}")
    return source.read()


def validate_runtime_archives(directory: Path, metadata: dict) -> None:
    """Bind the APK, runtime, provenance and complete relink inventory together."""
    try:
        with tarfile.open(
            directory / "pi-bun-android-arm64-runtime.tar.gz", "r:gz"
        ) as runtime:
            runtime_files = _archive_files(runtime)
            required_runtime = {
                "libbunembeddingpoc.so",
                "LICENSE.bun.md",
                "BUN-LINKED-NOTICES.txt",
                "BUN-RUNTIME-PROVENANCE.json",
            }
            if not required_runtime <= runtime_files.keys() or any(
                name not in required_runtime and not name.startswith("notices/")
                for name in runtime_files
            ):
                raise ValueError("Runtime archive member inventory is incomplete or unexpected")
            library = _archive_bytes(runtime, runtime_files["libbunembeddingpoc.so"])
            if hashlib.sha256(library).hexdigest() != metadata["librarySha256"]:
                raise ValueError("Downloaded runtime does not match the bundled APK")
            for name in ["LICENSE.bun.md", "BUN-LINKED-NOTICES.txt"]:
                if (
                    hashlib.sha256(_archive_bytes(runtime, runtime_files[name])).hexdigest()
                    != metadata["_noticeSha256"][name]
                ):
                    raise ValueError(f"APK and runtime archive disagree on {name}")
            provenance_bytes = _archive_bytes(
                runtime, runtime_files["BUN-RUNTIME-PROVENANCE.json"]
            )
            provenance = json.loads(provenance_bytes)
            if (
                provenance.get("schema") != 1
                or provenance.get("target") != "aarch64-linux-android28"
                or provenance.get("library_sha256") != metadata["librarySha256"]
                or provenance.get("bun", {}).get("tag") != "bun-v1.4.3"
                or provenance.get("bun", {}).get("commit")
                != "c6da4a4d3010e5553438c60f6bd76d981976867c"
                or provenance.get("bun", {}).get("source_archive_sha256")
                != BUN_SOURCE_SHA256
                or provenance.get("jsc", {}).get("commit")
                != "0c06faadf65bf8e8c8ad3a5a8aca83e1e9ed653f"
                or provenance.get("jsc", {}).get("source_archive_sha256")
                != JSC_SOURCE_SHA256
                or provenance.get("jsc", {}).get("prebuilt_archive_sha256")
                != JSC_PREBUILT_SHA256
                or provenance.get("dependency_source_archives")
                != DEPENDENCY_SOURCE_ARCHIVES
                or not provenance.get("inputs")
            ):
                raise ValueError("Runtime provenance identity or inventory is incomplete")

        with tarfile.open(
            directory / "pi-bun-android-arm64-relink.tar.gz", "r:gz"
        ) as relink:
            relink_files = _archive_files(relink)
            expected = set(provenance["relink_members"]) | {
                "LICENSE.bun.md",
                "BUN-LINKED-NOTICES.txt",
                "BUN-RUNTIME-PROVENANCE.json",
            }
            if relink_files.keys() != expected:
                raise ValueError("Relink archive does not match its complete member inventory")
            if _archive_bytes(
                relink, relink_files["BUN-RUNTIME-PROVENANCE.json"]
            ) != provenance_bytes:
                raise ValueError("Runtime and relink provenance records differ")
            for name in ["LICENSE.bun.md", "BUN-LINKED-NOTICES.txt"]:
                if (
                    hashlib.sha256(_archive_bytes(relink, relink_files[name])).hexdigest()
                    != metadata["_noticeSha256"][name]
                ):
                    raise ValueError(f"APK and relink archive disagree on {name}")
            required_relink = {
                "RELINKING.md",
                "scripts/relink.py",
                "sources/bun-v1.4.3-source.tar.gz",
                "sources/WebKit-0c06faadf65bf8e8c8ad3a5a8aca83e1e9ed653f.tar.gz",
            }
            if not required_relink <= relink_files.keys() or not any(
                name.startswith("inputs/")
                and Path(name).suffix in {".o", ".a", ".rlib"}
                for name in relink_files
            ):
                raise ValueError("Relink archive is missing source, instructions, or objects")
            for name, expected_member in provenance["relink_members"].items():
                data = _archive_bytes(relink, relink_files[name])
                if (
                    len(data) != expected_member.get("bytes")
                    or hashlib.sha256(data).hexdigest() != expected_member.get("sha256")
                ):
                    raise ValueError(f"Relink member does not match provenance: {name}")
            recorded_inputs = {
                item.get("member"): {
                    "bytes": item.get("bytes"),
                    "sha256": item.get("sha256"),
                }
                for item in provenance["inputs"]
            }
            if (
                not recorded_inputs
                or any(
                    provenance["relink_members"].get(name) != identity
                    for name, identity in recorded_inputs.items()
                )
            ):
                raise ValueError("Relink input records do not match the member inventory")
            semantic_identities = {
                "sources/bun-v1.4.3-source.tar.gz": BUN_SOURCE_SHA256,
                "sources/WebKit-0c06faadf65bf8e8c8ad3a5a8aca83e1e9ed653f.tar.gz": JSC_SOURCE_SHA256,
                "bridge/embedding.rs": provenance.get("bridge", {}).get("embedding.rs"),
                "bridge/jni.cpp": provenance.get("bridge", {}).get("jni.cpp"),
                "bridge/patched-runtime-lib.rs": provenance.get(
                    "patched_runtime_lib_sha256"
                ),
            }
            if any(
                not digest
                or provenance["relink_members"].get(name, {}).get("sha256") != digest
                for name, digest in semantic_identities.items()
            ):
                raise ValueError("Relink sources do not match pinned semantic provenance")
    except (tarfile.TarError, KeyError, json.JSONDecodeError) as error:
        raise ValueError("Invalid Bun runtime/relink archives") from error


def manifest(directory: Path, verify_android_signer: bool = False) -> Path:
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
    if verify_android_signer:
        validate_android_signature(directory / ANDROID_APK)
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
    parser.add_argument("--verify-android-signer", action="store_true")
    args = parser.parse_args()
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"][
        "version"
    ]
    validate_release_version(args.tag, version)
    if args.android_apk:
        validate_local_apk(args.android_apk)
        if args.verify_android_signer:
            validate_android_signature(args.android_apk)
        print("Verified bundled experimental Local APK:", args.android_apk)
    if args.directory:
        print(manifest(args.directory, args.verify_android_signer))


if __name__ == "__main__":
    main()
