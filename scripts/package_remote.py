#!/usr/bin/env python3
"""Publish the standalone SSH helper (Pi and license notices are embedded)."""
import argparse
import shutil
import subprocess
import sys
from pathlib import Path

from package_desktop import ROOT, TARGETS, check_binary


def asset_name(target: str) -> str:
    """Names match pi_core's checksum-verified SSH installer."""
    platform, arch, _ = TARGETS[target]
    suffix = ".exe" if platform == "windows" else ""
    return f"pi-desktop-remote-{platform}-{arch}{suffix}"


def package(binary: Path, target: str, output: Path) -> Path:
    """Stage only the native executable; --licenses prints its embedded notices."""
    check_binary(binary, target)
    output.mkdir(parents=True, exist_ok=True)
    destination = output / asset_name(target)
    shutil.copy2(binary, destination)
    destination.chmod(0o755)
    if "apple" in target:
        if sys.platform != "darwin":
            raise ValueError("macOS helper signing requires native codesign")
        subprocess.run(["codesign", "--force", "--sign", "-", str(destination)], check=True)
        subprocess.run(["codesign", "--verify", "--strict", str(destination)], check=True)
    return destination


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=TARGETS, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, default=ROOT / "dist")
    args = parser.parse_args()
    print(package(args.binary.resolve(), args.target, args.output.resolve()))


if __name__ == "__main__":
    main()
