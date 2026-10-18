import hashlib
import tempfile
import unittest
from pathlib import Path

import verify_published_release


REPOSITORY = "owner/repository"
TAG = "v0.0.20"
COMMIT = "1" * 40
TAG_OBJECT = "2" * 40


class PublishedReleaseTests(unittest.TestCase):
    def fixtures(self, directory):
        assets = []
        for index in range(12):
            path = directory / f"asset-{index}"
            path.write_bytes(f"payload-{index}".encode())
            assets.append(
                {
                    "name": path.name,
                    "state": "uploaded",
                    "size": path.stat().st_size,
                    "digest": "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest(),
                }
            )
        return {
            f"repos/{REPOSITORY}/git/ref/tags/{TAG}": {
                "ref": f"refs/tags/{TAG}",
                "object": {"type": "tag", "sha": TAG_OBJECT},
            },
            f"repos/{REPOSITORY}/git/tags/{TAG_OBJECT}": {
                "tag": TAG,
                "object": {"type": "commit", "sha": COMMIT},
            },
            f"repos/{REPOSITORY}/releases/tags/{TAG}": {
                "tag_name": TAG,
                "name": TAG,
                "draft": False,
                "prerelease": False,
                "published_at": "2026-01-01T00:00:00Z",
                "body": (Path("docs/releases") / f"{TAG}.md").read_text(),
                "html_url": "https://example.invalid/release",
                "assets": assets,
            },
        }

    def test_accepts_exact_immutable_tag_metadata_and_asset_hashes(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            data = self.fixtures(directory)
            result = verify_published_release.verify(
                REPOSITORY,
                TAG,
                COMMIT,
                TAG_OBJECT,
                directory,
                api_call=lambda endpoint: data[endpoint],
            )
            self.assertEqual(result["asset_count"], 12)

    def test_rejects_changed_tag_or_uploaded_asset(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            changed_tag = self.fixtures(directory)
            changed_tag[f"repos/{REPOSITORY}/git/ref/tags/{TAG}"]["object"][
                "sha"
            ] = "3" * 40
            with self.assertRaisesRegex(ValueError, "tag object changed"):
                verify_published_release.verify(
                    REPOSITORY,
                    TAG,
                    COMMIT,
                    TAG_OBJECT,
                    directory,
                    api_call=lambda endpoint: changed_tag[endpoint],
                )

        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            changed_asset = self.fixtures(directory)
            changed_asset[f"repos/{REPOSITORY}/releases/tags/{TAG}"]["assets"][0][
                "digest"
            ] = "sha256:" + "f" * 64
            with self.assertRaisesRegex(ValueError, "differs from validated input"):
                verify_published_release.verify(
                    REPOSITORY,
                    TAG,
                    COMMIT,
                    TAG_OBJECT,
                    directory,
                    api_call=lambda endpoint: changed_asset[endpoint],
                )


if __name__ == "__main__":
    unittest.main()
