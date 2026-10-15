#!/usr/bin/env python3
"""Bundle the unchanged installed Durable SDK with a trusted offline probe.

Install the pinned package-lock.json with npm ci --ignore-scripts first. No SDK
source rewriting, Node compatibility shims or compiled CLI subprocess is used.
"""

import argparse
import hashlib
import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORKSPACE = ROOT.parent.parent


def collect_notices(node_modules):
    parts = [
        "Android embedded Bun / Durable development probe",
        "Installed dependency notices; tree-shaking may omit some packages.",
        (WORKSPACE / "LICENSE").read_text(),
    ]
    visited = set()

    def packages(directory):
        for item in sorted(directory.iterdir()):
            if item.name.startswith(".") or not item.is_dir():
                continue
            if item.name.startswith("@"):
                packages(item)
                continue
            manifest = item / "package.json"
            if not manifest.exists():
                continue
            identity = item.resolve()
            if identity in visited:
                continue
            visited.add(identity)
            metadata = json.loads(manifest.read_text())
            parts.append(
                f"\n===== {metadata['name']} {metadata['version']} ({metadata.get('license', 'see license')}) ====="
            )
            for file in sorted(item.iterdir()):
                if file.is_file() and file.name.lower().startswith(
                    ("license", "copying", "notice")
                ):
                    parts.append(file.read_text(errors="replace"))
            if (item / "node_modules").is_dir():
                packages(item / "node_modules")

    packages(node_modules)
    return "\n\n".join(parts) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--out-dir", type=Path, default=Path("/tmp/pi-bun-embedding-poc/sdk-bundle")
    )
    args = parser.parse_args()
    args.out_dir.mkdir(parents=True, exist_ok=True)
    node_modules = ROOT / "node_modules"
    expected = json.loads((ROOT / "package.json").read_text())["dependencies"]
    versions = {}
    for name, version in expected.items():
        actual = json.loads((node_modules / name / "package.json").read_text())[
            "version"
        ]
        if actual != version:
            raise SystemExit(f"{name}: expected {version}, installed {actual}")
        versions[name] = actual
    bundle = args.out_dir / "durable-probe.bundle.mjs"
    subprocess.run(
        [
            "bun",
            "build",
            str(ROOT / "durable-probe.js"),
            "--target=bun",
            "--outfile=" + str(bundle),
        ],
        cwd=ROOT,
        check=True,
    )
    notices = args.out_dir / "NOTICES.txt"
    notices.write_text(collect_notices(node_modules))
    provenance = {
        "scope": "trusted offline Durable SDK/tool probe, no credentials or real inference",
        "dependencies": versions,
        "sourceSha256": hashlib.sha256(
            (ROOT / "durable-probe.js").read_bytes()
        ).hexdigest(),
        "bundleSha256": hashlib.sha256(bundle.read_bytes()).hexdigest(),
        "bundleBytes": bundle.stat().st_size,
        "dependencyLockSha256": hashlib.sha256(
            (ROOT / "package-lock.json").read_bytes()
        ).hexdigest(),
        "upstreamPackagesModified": False,
    }
    (args.out_dir / "bundle-provenance.json").write_text(
        json.dumps(provenance, indent=2) + "\n"
    )
    print(json.dumps(provenance, indent=2))


if __name__ == "__main__":
    main()
