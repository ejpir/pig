#!/usr/bin/env python3
"""Fetch pi's official release binary for the `bundled-backend` feature.

Downloads `pi-<platform>.tar.gz` (`.zip` on Windows) for the pi version the desktop
extension is written against (`PI_VERSION` in crates/pi_core/src/extension.rs),
checks it against packaging/pi-release.sha256, and writes `<out>/pi-<platform>.tar.gz`
with pi's executable and the files pi reads beside it at the top level, which is the
archive PI_DESKTOP_BACKEND_ARCHIVE names.

On macOS, PI_DESKTOP_CODESIGN_IDENTITY re-signs pi's executable for the hardened
runtime with packaging/macos/backend.entitlements before it is packed.

--notices also writes `<out>/pi-notices.txt`, the licenses of the npm packages
compiled into pi's executable: it installs pi's release lockfile with
`npm ci --ignore-scripts` (needs npm) and collects each package's license files.
"""
import argparse
import hashlib
import io
import json
import os
import platform as host
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import urllib.request
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PLATFORMS = ["darwin-arm64", "darwin-x64", "linux-x64", "linux-arm64", "windows-x64", "windows-arm64"]
RELEASES = "https://github.com/earendil-works/pi/releases/download"


def pinned_version() -> str:
    source = (ROOT / "crates/pi_core/src/extension.rs").read_text()
    return re.search(r'pub const PI_VERSION: &str = "([^"]+)";', source).group(1)


def pinned_hashes(version: str) -> dict[str, str]:
    lines = (ROOT / "packaging/pi-release.sha256").read_text().splitlines()
    header = re.match(r"# pi (\S+):", lines[0])
    if not header or header.group(1) != version:
        raise SystemExit(f"packaging/pi-release.sha256 does not pin pi {version}")
    return {name: digest for digest, name in (line.split() for line in lines[1:] if line.strip())}


def download(version: str, name: str, hashes: dict[str, str]) -> bytes:
    with urllib.request.urlopen(f"{RELEASES}/v{version}/{name}", timeout=300) as response:
        data = response.read()
    digest = hashlib.sha256(data).hexdigest()
    if digest != hashes[name]:
        raise SystemExit(f"{name}: SHA-256 {digest} is not the pinned {hashes[name]}")
    return data


def host_platform() -> str:
    system = {"Darwin": "darwin", "Linux": "linux", "Windows": "windows"}[host.system()]
    arch = {"arm64": "arm64", "aarch64": "arm64", "x86_64": "x64", "AMD64": "x64"}[host.machine()]
    return f"{system}-{arch}"


def unpack(data: bytes, name: str, folder: Path) -> None:
    if name.endswith(".zip"):
        with zipfile.ZipFile(io.BytesIO(data)) as zipped:
            zipped.extractall(folder)
        return
    # The tarballs hold one `pi/` folder; the zips hold its contents.
    with tempfile.TemporaryDirectory(prefix="pi-unpack-", dir=folder.parent) as temporary:
        with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as tar:
            tar.extractall(temporary, filter="data")
        for entry in (Path(temporary) / "pi").iterdir():
            entry.rename(folder / entry.name)


def notices(version: str, hashes: dict[str, str], output: Path) -> None:
    """Installs pi's own release lockfile, as pi's installer does, and collects licenses."""
    with tempfile.TemporaryDirectory(prefix="pi-notices-") as temporary:
        root = Path(temporary)
        for name, saved in [("pi-coding-agent-install-package.json", "package.json"),
                            ("pi-coding-agent-install-package-lock.json", "package-lock.json")]:
            (root / saved).write_bytes(download(version, name, hashes))
        npm = shutil.which("npm") or shutil.which("npm.cmd")
        if not npm:
            raise SystemExit("--notices needs npm")
        subprocess.run([npm, "ci", "--ignore-scripts", "--omit=dev", "--no-audit", "--no-fund"], cwd=root, check=True)
        lock = json.loads((root / "package-lock.json").read_text())
        parts = [f"Licenses of the npm packages compiled into pi {version}'s release executable.\n"]
        for path, entry in lock["packages"].items():
            folder = root / path
            if not path or entry.get("dev") or not (folder / "package.json").is_file():
                continue
            manifest = json.loads((folder / "package.json").read_text(encoding="utf-8"))
            files = []
            for item in sorted(folder.iterdir()):
                if re.match(r"(licen[cs]es?|copying|notice)", item.name, re.IGNORECASE):
                    files += sorted(item.iterdir()) if item.is_dir() else [item]
            text = "\n\n".join(file.read_text(encoding="utf-8", errors="replace").strip() for file in files)
            if not text:
                text = f"License: {manifest.get('license') or entry.get('license') or 'not stated'}"
                if manifest.get("author"):
                    text += f"\nAuthor: {json.dumps(manifest['author'])}"
            parts.append(f"\n== {manifest['name']}@{manifest['version']} ==\n\n{text}\n")
        output.write_text("".join(parts), encoding="utf-8")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--platform", choices=PLATFORMS, default=None, help="default: this machine")
    parser.add_argument("--out", type=Path, default=ROOT / "artifacts/pi")
    parser.add_argument("--notices", action="store_true", help="also write pi-notices.txt (needs npm)")
    args = parser.parse_args()
    target = args.platform or host_platform()
    version = pinned_version()
    hashes = pinned_hashes(version)
    name = f"pi-{target}.zip" if target.startswith("windows-") else f"pi-{target}.tar.gz"
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    folder = out / target
    shutil.rmtree(folder, ignore_errors=True)
    folder.mkdir()
    unpack(download(version, name, hashes), name, folder)
    program = folder / ("pi.exe" if target.startswith("windows-") else "pi")
    if not program.is_file():
        raise SystemExit(f"{name} has no {program.name}")
    if not target.startswith("windows-"):
        program.chmod(0o755)
    identity = os.environ.get("PI_DESKTOP_CODESIGN_IDENTITY")
    if identity and target.startswith("darwin-"):
        # Bun's engine compiles JavaScript at run time, which the hardened runtime
        # allows only with these entitlements.
        subprocess.run(["codesign", "--force", "--options", "runtime", "--timestamp", "--sign", identity,
                        "--entitlements", str(ROOT / "packaging/macos/backend.entitlements"), str(program)], check=True)
    if target == host_platform():
        reported = subprocess.run([str(program), "--version"], capture_output=True, text=True, timeout=60).stdout.strip()
        if reported != version:
            raise SystemExit(f"{program} reports {reported!r}, not {version}")
    archive = out / f"pi-{target}.tar.gz"
    with tarfile.open(archive, "w:gz", compresslevel=9) as tar:
        for entry in sorted(folder.iterdir()):
            tar.add(entry, arcname=entry.name)
    print(archive)
    if args.notices:
        notices(version, hashes, out / "pi-notices.txt")
        print(out / "pi-notices.txt")


if __name__ == "__main__":
    sys.exit(main())
