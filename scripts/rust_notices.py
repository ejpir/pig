#!/usr/bin/env python3
"""Generate or verify the pinned Android Rust dependency notice report."""

from __future__ import annotations

import argparse
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DESTINATION = ROOT / "licenses/RUST-DEPENDENCY-NOTICES.html"
VERSION = "cargo-about 0.9.2"


def normalized(data: bytes) -> bytes:
    lines = data.decode("utf-8").splitlines()
    return ("\n".join(line.rstrip(" \t") for line in lines) + "\n").encode()


def generate() -> bytes:
    version = subprocess.run(
        ["cargo", "about", "--version"],
        cwd=ROOT,
        check=True,
        text=True,
        stdout=subprocess.PIPE,
    ).stdout.strip()
    if version != VERSION:
        raise SystemExit(f"Expected {VERSION}, found {version}")
    with tempfile.TemporaryDirectory(prefix="pi-rust-notices-") as directory:
        output = Path(directory) / "notices.html"
        subprocess.run(
            [
                "cargo",
                "about",
                "generate",
                "--manifest-path",
                "crates/pi_android/Cargo.toml",
                "--features",
                "local-backend",
                "--target",
                "aarch64-linux-android",
                "--locked",
                "about.hbs",
                "-o",
                str(output),
            ],
            cwd=ROOT,
            check=True,
        )
        return normalized(output.read_bytes())


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    report = generate()
    if args.check:
        if not DESTINATION.is_file() or DESTINATION.read_bytes() != report:
            raise SystemExit(
                "Rust dependency notices are stale; run scripts/rust_notices.py"
            )
    else:
        DESTINATION.write_bytes(report)
        print(DESTINATION.relative_to(ROOT))


if __name__ == "__main__":
    main()
