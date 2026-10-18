import copy
import unittest
from unittest import mock

import release_recovery


REPOSITORY = "owner/repository"
TAG = "v1.2.3"
RUN_ID = "123"
COMMIT = "1" * 40
TAG_OBJECT = "2" * 40
VERIFY_STEP = release_recovery.VERIFY_STEP


def fixtures():
    run = {
        "name": "Desktop",
        "path": ".github/workflows/desktop.yml",
        "event": "push",
        "head_branch": TAG,
        "head_sha": COMMIT,
        "head_repository": {"full_name": REPOSITORY},
        "status": "completed",
        "conclusion": "failure",
        "run_attempt": 1,
        "created_at": "2026-01-02T00:00:00Z",
    }
    jobs = [
        {"name": name, "conclusion": "success", "run_attempt": 1, "steps": []}
        for name in sorted(release_recovery.REQUIRED_JOBS)
    ]
    jobs.append(
        {
            "name": f"Publish / {TAG}",
            "conclusion": "failure",
            "run_attempt": 1,
            "steps": [
                {"name": "Run actions/checkout@v4", "conclusion": "success"},
                {"name": "Run actions/setup-python@v5", "conclusion": "success"},
                {
                    "name": "Run actions/download-artifact@v4",
                    "conclusion": "success",
                },
                {"name": VERIFY_STEP, "conclusion": "failure"},
                {"name": "Publish GitHub release", "conclusion": "skipped"},
            ],
        }
    )
    artifacts = []
    for index, name in enumerate(sorted(release_recovery.REQUIRED_ARTIFACTS), 1):
        artifacts.append(
            {
                "id": index,
                "name": name,
                "expired": False,
                "size_in_bytes": index,
                "digest": "sha256:" + format(index, "064x"),
                "workflow_run": {
                    "id": int(RUN_ID),
                    "head_branch": TAG,
                    "head_sha": COMMIT,
                },
            }
        )
    return {
        f"repos/{REPOSITORY}/actions/runs/{RUN_ID}": run,
        f"repos/{REPOSITORY}": {"default_branch": "main"},
        f"repos/{REPOSITORY}/actions/workflows/desktop.yml/runs?branch=main&event=push&status=success&per_page=100": {
            "workflow_runs": [
                {
                    "id": 99,
                    "head_sha": COMMIT,
                    "head_branch": "main",
                    "run_attempt": 1,
                    "created_at": "2026-01-01T00:00:00Z",
                }
            ]
        },
        f"repos/{REPOSITORY}/git/ref/tags/{TAG}": {
            "ref": f"refs/tags/{TAG}",
            "object": {"type": "tag", "sha": TAG_OBJECT},
        },
        f"repos/{REPOSITORY}/git/tags/{TAG_OBJECT}": {
            "tag": TAG,
            "object": {"type": "commit", "sha": COMMIT},
        },
        f"repos/{REPOSITORY}/actions/runs/{RUN_ID}/jobs?per_page=100&page=1": {
            "total_count": len(jobs),
            "jobs": jobs,
        },
        f"repos/{REPOSITORY}/actions/runs/{RUN_ID}/artifacts?per_page=100&page=1": {
            "total_count": len(artifacts),
            "artifacts": artifacts,
        },
    }


def validate(data, run_log=None):
    missing = []
    if run_log is None:
        run_log = "\n".join(
            f"Download action repository '{action}' (SHA:{commit})"
            for action, commit in release_recovery.EXPECTED_ACTION_COMMITS.items()
        )
    result = release_recovery.validate(
        REPOSITORY,
        TAG,
        RUN_ID,
        TAG_OBJECT,
        api_call=lambda endpoint: data[endpoint],
        missing_release=lambda repository, tag: missing.append((repository, tag)),
        run_log=run_log,
    )
    return result, missing


class ReleaseRecoveryTests(unittest.TestCase):
    def test_accepts_one_first_attempt_with_exact_tag_jobs_and_artifacts(self):
        result, missing = validate(fixtures())
        self.assertEqual(result["commit"], COMMIT)
        self.assertEqual(result["tag_object"], TAG_OBJECT)
        self.assertEqual(result["branch_run_id"], "99")
        self.assertEqual(len(result["artifact_ids"].split(",")), 6)
        self.assertEqual(missing, [(REPOSITORY, TAG)])

    def test_release_lookup_fails_closed_except_for_explicit_404(self):
        with mock.patch.object(
            release_recovery.subprocess,
            "run",
            return_value=mock.Mock(returncode=1, stderr="gh: outage (HTTP 500)"),
        ):
            with self.assertRaisesRegex(RuntimeError, "Could not inspect"):
                release_recovery.require_missing_release(REPOSITORY, TAG)
        with mock.patch.object(
            release_recovery.subprocess,
            "run",
            return_value=mock.Mock(returncode=1, stderr="gh: Not Found (HTTP 404)"),
        ):
            release_recovery.require_missing_release(REPOSITORY, TAG)
        with mock.patch.object(
            release_recovery.subprocess,
            "run",
            return_value=mock.Mock(returncode=0, stderr=""),
        ):
            with self.assertRaisesRegex(ValueError, "already exists"):
                release_recovery.require_missing_release(REPOSITORY, TAG)

    def test_rejects_reruns_and_duplicate_jobs(self):
        rerun = fixtures()
        rerun[f"repos/{REPOSITORY}/actions/runs/{RUN_ID}"]["run_attempt"] = 2
        with self.assertRaisesRegex(ValueError, "first failed"):
            validate(rerun)

        duplicate = fixtures()
        response = duplicate[
            f"repos/{REPOSITORY}/actions/runs/{RUN_ID}/jobs?per_page=100&page=1"
        ]
        response["jobs"].append(copy.deepcopy(response["jobs"][0]))
        response["total_count"] += 1
        with self.assertRaisesRegex(ValueError, "duplicate job"):
            validate(duplicate)

    def test_rejects_any_failure_other_than_artifact_validation(self):
        data = fixtures()
        jobs = data[
            f"repos/{REPOSITORY}/actions/runs/{RUN_ID}/jobs?per_page=100&page=1"
        ]["jobs"]
        publish = next(job for job in jobs if job["name"].startswith("Publish /"))
        publish["steps"][2]["conclusion"] = "failure"
        with self.assertRaisesRegex(ValueError, "fail only at artifact validation"):
            validate(data)

    def test_rejects_missing_branch_confirmation_and_artifact_origin_change(self):
        branch = fixtures()
        branch[
            f"repos/{REPOSITORY}/actions/workflows/desktop.yml/runs?branch=main&event=push&status=success&per_page=100"
        ]["workflow_runs"] = []
        with self.assertRaisesRegex(ValueError, "default-branch run"):
            validate(branch)

        origin = fixtures()
        response = origin[
            f"repos/{REPOSITORY}/actions/runs/{RUN_ID}/artifacts?per_page=100&page=1"
        ]
        response["artifacts"][0]["workflow_run"]["head_sha"] = "f" * 40
        with self.assertRaisesRegex(ValueError, "Invalid or unbound"):
            validate(origin)

    def test_rejects_unreviewed_resolved_action_commit(self):
        log = "\n".join(
            f"Download action repository '{action}' (SHA:{'f' * 40 if action == 'actions/checkout@v4' else commit})"
            for action, commit in release_recovery.EXPECTED_ACTION_COMMITS.items()
        )
        with self.assertRaisesRegex(ValueError, "exact reviewed action commits"):
            validate(fixtures(), log)

    def test_rejects_changed_tag_or_ambiguous_artifacts(self):
        changed = fixtures()
        changed[f"repos/{REPOSITORY}/git/ref/tags/{TAG}"]["object"]["sha"] = "3" * 40
        with self.assertRaisesRegex(ValueError, "original annotated tag"):
            validate(changed)

        duplicate = fixtures()
        response = duplicate[
            f"repos/{REPOSITORY}/actions/runs/{RUN_ID}/artifacts?per_page=100&page=1"
        ]
        response["artifacts"].append(copy.deepcopy(response["artifacts"][0]))
        response["total_count"] += 1
        with self.assertRaisesRegex(ValueError, "duplicate artifact"):
            validate(duplicate)

        expired = fixtures()
        response = expired[
            f"repos/{REPOSITORY}/actions/runs/{RUN_ID}/artifacts?per_page=100&page=1"
        ]
        response["artifacts"][0]["expired"] = True
        with self.assertRaisesRegex(ValueError, "Invalid or unbound"):
            validate(expired)


if __name__ == "__main__":
    unittest.main()
