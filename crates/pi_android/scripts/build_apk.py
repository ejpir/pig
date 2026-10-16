#!/usr/bin/env python3
"""Build Pi for Android as an APK, without Gradle.

    python3 crates/pi_android/scripts/build_apk.py [--debug] [--out dist/pi.apk]

Uses gpui_android's builder (crates/gpui_android/scripts/build_apk.py), which
lists what it needs: a JDK and the Android SDK with the NDK.
"""

import argparse
import hashlib
import json
import os
import struct
import subprocess
import sys
from pathlib import Path

sys.path.insert(
    0, str(Path(__file__).resolve().parents[2] / "gpui_android" / "scripts")
)

import build_apk

PI = build_apk.App(
    package="dev.pi.android",
    label="Pi",
    library="pi_android",
    cargo=["-p", "pi_android", "--lib"],
    output="",
    # SSH to the computer; questions and finished sessions arrive as notifications.
    permissions=(
        "android.permission.INTERNET",
        "android.permission.POST_NOTIFICATIONS",
        "android.permission.CAMERA",
    ),
    profileable=True,
    qr_scanner=True,
    deep_links=(("pi", "pair"),),
    launcher=(Path(__file__).resolve().parents[1] / "assets" / "launcher", 0xFF161D27),
    licenses=tuple(
        build_apk.WORKSPACE / "licenses" / name
        for name in (
            "IBM-PLEX-OFL.txt",
            "COMMIT-MONO-OFL.txt",
            "RQUICKJS-MIT.txt",
            "QUICKJS-NG-MIT.txt",
            "ROXMLTREE-MIT.txt",
            "SVGTYPES-MIT.txt",
            "KURBO-MIT.txt",
            "LYON-MIT.txt",
        )
    ),
)


def configure_local(library: Path, target: str) -> None:
    """Fail closed rather than publish a Local UI without its real runtime."""
    library = library.resolve()
    if library.name != "libbunembeddingpoc.so" or not library.is_file():
        raise ValueError("Expected the source-linked libbunembeddingpoc.so")
    with library.open("rb") as source:
        header = source.read(64)
    machine = {"aarch64-linux-android": 183, "x86_64-linux-android": 62}.get(target)
    if (
        len(header) < 64
        or header[:6] != b"\x7fELF\x02\x01"
        or struct.unpack_from("<HH", header, 16) != (3, machine)
    ):
        raise ValueError("Bun must be an architecture-matching ELF64 shared library")
    symbols = subprocess.check_output(
        ["readelf", "--dyn-syms", "--wide", str(library)], text=True
    )
    if not all(
        name in symbols for name in ("pi_bun_init", "pi_bun_eval", "pi_bun_pump")
    ):
        raise ValueError("Missing the source embedding ABI")
    local = build_apk.WORKSPACE / "crates/pi_local"
    subprocess.run(["bun", "run", "build"], cwd=local, check=True)
    subprocess.run(
        [
            sys.executable,
            str(build_apk.WORKSPACE / "experiments/bun-embedding/build_local.py"),
        ],
        check=True,
    )
    license_path = library.parent / "LICENSE.bun.md"
    if not license_path.is_file():
        raise ValueError(
            "Place the pinned Bun LICENSE.bun.md beside the shared library"
        )
    version, _ = build_apk.workspace_version()
    with library.open("rb") as source:
        digest = hashlib.file_digest(source, "sha256").hexdigest()
    metadata = {
        "version": version,
        "status": "experimental",
        "engine": "bun",
        "bun": "1.4.3",
        "sdk": "1.3.0",
        "abi": build_apk.TARGET_ABIS[target],
        "librarySha256": digest,
        "bundleSha256": hashlib.sha256(
            (local / "generated/bun-local.mjs").read_bytes()
        ).hexdigest(),
    }
    PI.assets += (
        ("local-runtime.json", json.dumps(metadata, sort_keys=True).encode()),
        ("bun-local.mjs", (local / "generated/bun-local.mjs").read_bytes()),
    )
    PI.cargo += ["--features", "local-backend"]
    PI.native_libraries = (library,)
    linked_notices = library.parent / "BUN-LINKED-NOTICES.txt"
    if not linked_notices.is_file():
        raise ValueError(
            "Bun runtime bundle must include BUN-LINKED-NOTICES.txt; use build_runtime.py"
        )
    PI.licenses += (
        license_path,
        linked_notices,
        local / "generated/BUN-DURABLE-NOTICES.txt",
        local / "generated/LOCAL-AUTH-NPM-NOTICES.txt",
    )


def main() -> None:
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument(
        "--bun-library", type=Path, default=os.environ.get("PI_ANDROID_BUN_LIBRARY")
    )
    parser.add_argument("--bun-backend-test", action="store_true")
    parser.add_argument(
        "--ssh-only",
        action="store_true",
        help="Developer-only build without experimental Local",
    )
    options, remaining = parser.parse_known_args()
    sys.argv = [sys.argv[0], *remaining]
    target_parser = argparse.ArgumentParser(add_help=False)
    target_parser.add_argument("--target", default=build_apk.TARGET)
    target, _ = target_parser.parse_known_args(remaining)
    if "--help" in remaining or "-h" in remaining:
        parser.print_help()
        build_apk.main(PI, "pi.apk")
        return
    fixture = options.bun_backend_test
    if fixture and options.ssh_only:
        raise SystemExit("Local fixtures cannot use --ssh-only")
    if not options.ssh_only and "--ui-test" not in remaining:
        if not options.bun_library:
            raise SystemExit(
                "Local APK requires --bun-library or PI_ANDROID_BUN_LIBRARY (see experiments/bun-embedding); use --ssh-only only for developer builds"
            )
        configure_local(options.bun_library, target.target)
    if fixture:
        PI.package = "dev.pi.android.quickjstest"
        PI.label = "Pi Local (experimental)"
        PI.debuggable = True
        PI.cargo += ["--features", "bun-backend-test"]
        build_apk.main(PI, "pi-bun-local-test.apk")
    elif "--ui-test" in sys.argv:
        sys.argv.remove("--ui-test")
        PI.package = "dev.pi.android.uitest"
        PI.label = "Pi UI tests"
        PI.debuggable = True
        PI.cargo += ["--features", "ui-test"]
        build_apk.main(PI, "pi-ui-test.apk")
    else:
        build_apk.main(PI, "pi.apk")


if __name__ == "__main__":
    main()
