#!/usr/bin/env python3
"""Validate an immutable failed tag run before CI retries release publication."""

import argparse
import json
import re
import subprocess
from collections.abc import Callable

TAG = re.compile(r"v\d+\.\d+\.\d+")
SHA = re.compile(r"[0-9a-f]{40}")
REQUIRED_JOBS = {
    "Tests and lints (Linux)",
    "Build / embedded Bun Android ARM64",
    "Package / linux-amd64",
    "Package / linux-arm64",
    "Package / macos-arm64",
    "Package / windows-amd64",
    "Package / android-arm64",
}
REQUIRED_ARTIFACTS = {
    "pi-desktop-linux-amd64",
    "pi-desktop-linux-arm64",
    "pi-desktop-macos-arm64",
    "pi-desktop-windows-amd64",
    "pi-desktop-android-arm64",
    "pi-desktop-bun-android-arm64",
}
EXPECTED_ACTION_COMMITS = {
    "actions/cache@v4": "0057852bfaa89a56745cba8c7296529d2fc39830",
    "actions/checkout@v4": "11d5960a326750d5838078e36cf38b85af677262",
    "actions/download-artifact@v4": "d3f86a106a0bac45b974a628896c90dbdf5c8093",
    "actions/setup-java@v4": "cf277c60eb25467037889841efdb72551f06f6c3",
    "actions/setup-node@v4": "49933ea5288caeca8642d1e84afbd3f7d6820020",
    "actions/setup-python@v5": "a26af69be951a213d495a4c3e4e4022e16d87065",
    "actions/upload-artifact@v4": "ea165f8d65b6e75b540449e92b4886f43607fa02",
    "dtolnay/rust-toolchain@master": "683b588c487a69d01f6c6ddf40f036ff682cc2c8",
    "ilammy/msvc-dev-cmd@v1": "0b201ec74fa43914dc39ae48a89fd1d8cb592756",
    "oven-sh/setup-bun@v2": "0c5077e51419868618aeaa5fe8019c62421857d6",
    "Swatinem/rust-cache@v2": "6323deb102c322ba6fcbdcafc7e3dddab59af2b6",
}
VERIFY_STEP = (
    "Verify desktop packages/helpers, bundled Local APK and runtime/relink archives"
)


def api(endpoint: str) -> dict:
    result = subprocess.run(
        ["gh", "api", endpoint],
        check=True,
        stdout=subprocess.PIPE,
        text=True,
    )
    return json.loads(result.stdout)


def fetch_run_log(repository: str, run_id: str) -> str:
    result = subprocess.run(
        ["gh", "run", "view", run_id, "--repo", repository, "--log"],
        check=True,
        stdout=subprocess.PIPE,
        text=True,
    )
    return result.stdout


def validate_resolved_actions(log: str) -> None:
    resolved: dict[str, str] = {}
    for action, commit in re.findall(
        r"Download action repository '([^']+)' \(SHA:([0-9a-f]{40})\)", log
    ):
        previous = resolved.setdefault(action, commit)
        if previous != commit:
            raise ValueError(f"Action resolved to multiple commits: {action}")
    if resolved != EXPECTED_ACTION_COMMITS:
        raise ValueError(
            "Source run did not use the exact reviewed action commits: "
            f"expected={EXPECTED_ACTION_COMMITS}, actual={resolved}"
        )


def require_missing_release(repository: str, tag: str) -> None:
    result = subprocess.run(
        ["gh", "api", f"repos/{repository}/releases/tags/{tag}"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        text=True,
    )
    if result.returncode == 0:
        raise ValueError(f"Release {tag} already exists; inspect it instead of recovering")
    if "HTTP 404" not in result.stderr:
        raise RuntimeError(f"Could not inspect release {tag}: {result.stderr.strip()}")


def unique_by_name(items: list[dict], kind: str) -> dict[str, dict]:
    named: dict[str, dict] = {}
    for item in items:
        name = item.get("name")
        if not isinstance(name, str) or name in named:
            raise ValueError(f"Missing or duplicate {kind} name: {name!r}")
        named[name] = item
    return named


def validate(
    repository: str,
    tag: str,
    run_id: str,
    expected_tag_object: str,
    *,
    api_call: Callable[[str], dict] = api,
    missing_release: Callable[[str, str], None] = require_missing_release,
    run_log: str | None = None,
) -> dict[str, str]:
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository):
        raise ValueError("Invalid repository")
    if TAG.fullmatch(tag) is None:
        raise ValueError("Recovery tag must be an exact vMAJOR.MINOR.PATCH")
    if not run_id.isascii() or not run_id.isdigit():
        raise ValueError("Source run ID must contain only decimal digits")
    if SHA.fullmatch(expected_tag_object) is None:
        raise ValueError("Expected annotated tag object must be a full SHA-1")

    run = api_call(f"repos/{repository}/actions/runs/{run_id}")
    if any(
        (
            run.get("name") != "Desktop",
            run.get("path") != ".github/workflows/desktop.yml",
            run.get("event") != "push",
            run.get("head_branch") != tag,
            run.get("status") != "completed",
            run.get("conclusion") != "failure",
            run.get("run_attempt") != 1,
            (run.get("head_repository") or {}).get("full_name") != repository,
        )
    ):
        raise ValueError("Source run is not the first failed Desktop tag workflow attempt")
    commit = run.get("head_sha")
    if not isinstance(commit, str) or SHA.fullmatch(commit) is None:
        raise ValueError("Source run has no exact commit identity")

    repository_data = api_call(f"repos/{repository}")
    default_branch = repository_data.get("default_branch")
    branch_runs = api_call(
        f"repos/{repository}/actions/workflows/desktop.yml/runs"
        f"?branch={default_branch}&event=push&status=success&per_page=100"
    ).get("workflow_runs", [])
    confirmed = [
        candidate
        for candidate in branch_runs
        if candidate.get("head_sha") == commit
        and candidate.get("head_branch") == default_branch
        and candidate.get("run_attempt") == 1
        and candidate.get("created_at", "") <= run.get("created_at", "")
    ]
    if len(confirmed) != 1:
        raise ValueError("Tag commit lacks one successful first-attempt default-branch run")

    reference = api_call(f"repos/{repository}/git/ref/tags/{tag}")
    reference_object = reference.get("object") or {}
    if (
        reference.get("ref") != f"refs/tags/{tag}"
        or reference_object.get("type") != "tag"
        or reference_object.get("sha") != expected_tag_object
    ):
        raise ValueError("Recovery tag is not the expected original annotated tag object")
    annotated = api_call(f"repos/{repository}/git/tags/{expected_tag_object}")
    annotated_target = annotated.get("object") or {}
    if (
        annotated.get("tag") != tag
        or annotated_target.get("type") != "commit"
        or annotated_target.get("sha") != commit
    ):
        raise ValueError("Annotated tag, source workflow, and commit do not agree")

    jobs_response = api_call(
        f"repos/{repository}/actions/runs/{run_id}/jobs?per_page=100&page=1"
    )
    jobs = jobs_response.get("jobs", [])
    if jobs_response.get("total_count") != len(jobs) or len(jobs) > 100:
        raise ValueError("Source run job inventory is incomplete")
    named_jobs = unique_by_name(jobs, "job")
    expected_jobs = REQUIRED_JOBS | {f"Publish / {tag}"}
    if set(named_jobs) != expected_jobs:
        raise ValueError("Source run job inventory is unexpected")
    for name in REQUIRED_JOBS:
        job = named_jobs[name]
        if job.get("conclusion") != "success" or job.get("run_attempt") != 1:
            raise ValueError(f"Required source job was not successful: {name}")

    publish = named_jobs[f"Publish / {tag}"]
    if publish.get("conclusion") != "failure" or publish.get("run_attempt") != 1:
        raise ValueError("The first publication job did not fail")
    steps = unique_by_name(publish.get("steps", []), "publication step")
    required_steps = {
        "Run actions/checkout@v4": "success",
        "Run actions/setup-python@v5": "success",
        "Run actions/download-artifact@v4": "success",
        VERIFY_STEP: "failure",
        "Publish GitHub release": "skipped",
    }
    if any(steps.get(name, {}).get("conclusion") != result for name, result in required_steps.items()):
        raise ValueError("Publication did not fail only at artifact validation")
    failed_steps = [
        name for name, step in steps.items() if step.get("conclusion") == "failure"
    ]
    if failed_steps != [VERIFY_STEP]:
        raise ValueError(f"Unexpected failed publication steps: {failed_steps}")

    artifacts_response = api_call(
        f"repos/{repository}/actions/runs/{run_id}/artifacts?per_page=100&page=1"
    )
    all_artifacts = artifacts_response.get("artifacts", [])
    if artifacts_response.get("total_count") != len(all_artifacts) or len(all_artifacts) > 100:
        raise ValueError("Source run artifact inventory is incomplete")
    artifacts = [
        artifact
        for artifact in all_artifacts
        if str(artifact.get("name", "")).startswith("pi-desktop-")
    ]
    named_artifacts = unique_by_name(artifacts, "artifact")
    if set(named_artifacts) != REQUIRED_ARTIFACTS:
        raise ValueError("Release artifact container inventory is incomplete or duplicated")
    for name, artifact in named_artifacts.items():
        origin = artifact.get("workflow_run") or {}
        digest = artifact.get("digest")
        if any(
            (
                artifact.get("expired") is not False,
                not isinstance(artifact.get("id"), int),
                not isinstance(artifact.get("size_in_bytes"), int),
                artifact.get("size_in_bytes", 0) <= 0,
                not isinstance(digest, str),
                re.fullmatch(r"sha256:[0-9a-f]{64}", digest or "") is None,
                origin.get("id") != int(run_id),
                origin.get("head_branch") != tag,
                origin.get("head_sha") != commit,
            )
        ):
            raise ValueError(f"Invalid or unbound release artifact: {name}")

    validate_resolved_actions(
        run_log if run_log is not None else fetch_run_log(repository, run_id)
    )
    missing_release(repository, tag)
    sorted_artifacts = [named_artifacts[name] for name in sorted(REQUIRED_ARTIFACTS)]
    return {
        "commit": commit,
        "tag_object": expected_tag_object,
        "branch_run_id": str(confirmed[0]["id"]),
        "artifact_ids": ",".join(str(artifact["id"]) for artifact in sorted_artifacts),
        "artifact_digests": json.dumps(
            {artifact["name"]: artifact["digest"] for artifact in sorted_artifacts},
            sort_keys=True,
            separators=(",", ":"),
        ),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--run-id", required=True)
    parser.add_argument("--tag-object", required=True)
    parser.add_argument("--github-output", action="store_true")
    args = parser.parse_args()
    result = validate(args.repository, args.tag, args.run_id, args.tag_object)
    if args.github_output:
        for name, value in result.items():
            print(f"{name}={value}")
    else:
        print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
