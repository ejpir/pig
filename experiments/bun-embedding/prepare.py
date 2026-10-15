#!/usr/bin/env python3
"""Prepare an isolated, pinned Bun source build plus the experimental JNI bridge."""

import argparse
import hashlib
import shutil
import subprocess
import tarfile
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent
TAG = "bun-v1.4.3"
COMMIT = "c6da4a4d3010e5553438c60f6bd76d981976867c"
SOURCE_SHA256 = "2366bc56d092f8535bbe5827788e20d987fbc96798de4d683e0ca78ea286ebf4"


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--work-dir", type=Path, default=Path("/tmp/pi-bun-embedding-poc"))
    args = p.parse_args()
    work = args.work_dir.resolve()
    downloads = work / "downloads"
    downloads.mkdir(parents=True, exist_ok=True)
    archive = downloads / (TAG + "-source.tar.gz")
    if not archive.exists():
        with (
            urllib.request.urlopen(
                "https://codeload.github.com/oven-sh/bun/tar.gz/refs/tags/" + TAG,
                timeout=60,
            ) as response,
            archive.open("wb") as output,
        ):
            shutil.copyfileobj(response, output)
    if hashlib.sha256(archive.read_bytes()).hexdigest() != SOURCE_SHA256:
        raise SystemExit("Pinned source archive checksum mismatch")
    source = work / "source" / ("bun-" + TAG)
    if not source.exists():
        with tarfile.open(archive) as tar:
            prefix = "bun-" + TAG
            excluded = tuple(
                prefix + "/" + name + "/"
                for name in ["test", "bench", "docs", ".buildkite", ".github"]
            )
            entries = [
                item for item in tar.getmembers() if not item.name.startswith(excluded)
            ]
            tar.extractall(work / "source", members=entries, filter="data")
    if not (source / ".git").exists():
        subprocess.run(["git", "init", "-q", str(source)], check=True)
    runtime = source / "src/runtime"
    (runtime / "pi_embedding.rs").write_bytes(
        (ROOT / "native/embedding.rs").read_bytes()
    )
    lib = runtime / "lib.rs"
    contents = lib.read_text()
    if "mod pi_embedding;" not in contents:
        if "mod bin_entry;" not in contents:
            raise SystemExit("Pinned runtime layout changed")
        lib.write_text(
            contents.replace("mod bin_entry;", "mod bin_entry;\nmod pi_embedding;", 1)
        )
    print("Prepared", source)
    print("Base commit:", COMMIT, "(plus explicit pi_embedding.rs module)")
    print(
        "Set GIT_SHA="
        + COMMIT
        + " during the source build; never call Bun main from JNI."
    )


if __name__ == "__main__":
    main()
