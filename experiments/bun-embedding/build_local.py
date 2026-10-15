#!/usr/bin/env python3
"""Bundle the shared Durable runner and trusted Android host using unchanged SDK packages."""

import hashlib
import json
import subprocess
from pathlib import Path

from build_sdk_probe import collect_notices

ROOT = Path(__file__).resolve().parent
WORKSPACE = ROOT.parent.parent
OUT = WORKSPACE / "crates/pi_local/generated"
SDK = WORKSPACE / "backend/durable"
HOST = SDK / "android"


def main():
    expected = json.loads((SDK / "package.json").read_text())["dependencies"]
    for name, version in expected.items():
        if (
            json.loads((SDK / "node_modules" / name / "package.json").read_text())[
                "version"
            ]
            != version
        ):
            raise SystemExit("Mismatched pinned SDK: " + name)
    OUT.mkdir(exist_ok=True)
    subprocess.run(
        [
            "bun",
            "build",
            str(HOST / "main.js"),
            "--target=bun",
            "--outfile=" + str(OUT / "bun-local.mjs"),
        ],
        cwd=SDK,
        check=True,
    )
    (OUT / "BUN-DURABLE-NOTICES.txt").write_text(collect_notices(SDK / "node_modules"))
    paths = [
        *sorted(HOST.glob("*.js")),
        SDK / "package.json",
        SDK / "package-lock.json",
        *sorted((WORKSPACE / "backend/durable/src").glob("*.ts")),
        ROOT / "build_local.py",
        ROOT / "build_sdk_probe.py",
        OUT / "bun-local.mjs",
        OUT / "BUN-DURABLE-NOTICES.txt",
    ]
    (OUT / "bun-manifest.json").write_text(
        json.dumps(
            {
                "version": 1,
                "files": {
                    str(p.relative_to(WORKSPACE)): hashlib.sha256(
                        p.read_bytes()
                    ).hexdigest()
                    for p in paths
                },
            },
            indent=2,
        )
        + "\n"
    )


if __name__ == "__main__":
    main()
