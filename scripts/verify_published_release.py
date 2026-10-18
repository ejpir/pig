#!/usr/bin/env python3
"""Verify a CI-published GitHub release against local validated artifacts."""

import argparse
import hashlib
import json
import re
import subprocess
from collections.abc import Callable
from pathlib import Path

SHA = re.compile(r"[0-9a-f]{40}")


def api(endpoint: str) -> dict:
    result = subprocess.run(
        ["gh", "api", endpoint],
        check=True,
        stdout=subprocess.PIPE,
        text=True,
    )
    return json.loads(result.stdout)


def verify(
    repository: str,
    tag: str,
    commit: str,
    tag_object: str,
    directory: Path,
    *,
    api_call: Callable[[str], dict] = api,
) -> dict:
    if SHA.fullmatch(commit) is None or SHA.fullmatch(tag_object) is None:
        raise ValueError("Commit and tag object must be full SHA-1 identities")
    reference = api_call(f"repos/{repository}/git/ref/tags/{tag}")
    reference_object = reference.get("object") or {}
    if (
        reference.get("ref") != f"refs/tags/{tag}"
        or reference_object.get("type") != "tag"
        or reference_object.get("sha") != tag_object
    ):
        raise ValueError("Annotated release tag object changed during publication")
    annotated = api_call(f"repos/{repository}/git/tags/{tag_object}")
    if (
        annotated.get("tag") != tag
        or (annotated.get("object") or {}).get("type") != "commit"
        or (annotated.get("object") or {}).get("sha") != commit
    ):
        raise ValueError("Annotated release tag target changed during publication")

    release = api_call(f"repos/{repository}/releases/tags/{tag}")
    notes = Path("docs/releases") / f"{tag}.md"
    if any(
        (
            release.get("tag_name") != tag,
            release.get("name") != tag,
            release.get("draft") is not False,
            release.get("prerelease") is not False,
            not release.get("published_at"),
            release.get("body", "").strip() != notes.read_text().strip(),
        )
    ):
        raise ValueError("Published release metadata or notes do not match")

    expected = {path.name: path for path in directory.iterdir() if path.is_file()}
    assets = release.get("assets", [])
    actual = {}
    for asset in assets:
        name = asset.get("name")
        if not isinstance(name, str) or name in actual:
            raise ValueError(f"Missing or duplicate release asset name: {name!r}")
        actual[name] = asset
    if set(actual) != set(expected) or len(expected) != 12:
        raise ValueError("Published release must contain exactly the 12 validated assets")
    for name, path in expected.items():
        with path.open("rb") as source:
            digest = "sha256:" + hashlib.file_digest(source, "sha256").hexdigest()
        asset = actual[name]
        if any(
            (
                asset.get("state") != "uploaded",
                asset.get("size") != path.stat().st_size,
                asset.get("digest") != digest,
            )
        ):
            raise ValueError(f"Published release asset differs from validated input: {name}")
    return {"url": release.get("html_url"), "asset_count": len(actual)}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--tag-object", required=True)
    parser.add_argument("--directory", type=Path, required=True)
    args = parser.parse_args()
    print(
        json.dumps(
            verify(
                args.repository,
                args.tag,
                args.commit,
                args.tag_object,
                args.directory,
            ),
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
