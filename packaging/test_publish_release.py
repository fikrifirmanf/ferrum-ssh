"""Hermetic release tests; GitHub CLI is replaced at the external boundary."""
import copy
import hashlib
import importlib.util
import tempfile
import unittest
from unittest import mock
from pathlib import Path

spec = importlib.util.spec_from_file_location("publisher", Path(__file__).with_name("publish_release.py"))
publisher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(publisher)
SHA = "a" * 40
DIGEST = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
NAMES = [
    "Ferrum-1.2.3-macos-arm64.dmg", "Ferrum-1.2.3-macos-x86_64.dmg",
    "Ferrum-1.2.3-linux-x86_64.tar.gz", "Ferrum-1.2.3-windows-x86_64.zip",
    "Ferrum-1.2.3-windows-x86_64-setup.exe",
]


class FakeGitHub:
    def __init__(self, directory):
        self.directory = directory
        self.calls = []
        self.existing = None
        self.draft = None
        self.tag_sha = SHA
        self.annotated = False
        self.corrupt_upload = False
        self.fail_upload = False

    def release(self, tag):
        self.calls.append(("release", tag))
        return copy.deepcopy(self.existing)

    def api(self, method, path, payload=None):
        self.calls.append((method, path, copy.deepcopy(payload)))
        if "/git/ref/tags/" in path:
            return {"object": {"type": "tag" if self.annotated else "commit", "sha": self.tag_sha}}
        if "/git/tags/" in path:
            return {"object": {"type": "commit", "sha": self.tag_sha}}
        if method == "POST":
            self.draft = {"id": 7, "html_url": "https://github.com/fikrifirmanf/ferrum-ssh/releases/tag/" + payload["tag_name"],
                          "assets": [], **copy.deepcopy(payload)}
            return copy.deepcopy(self.draft)
        if method == "GET":
            return copy.deepcopy(self.draft)
        if method == "PATCH":
            self.draft.update(payload)
            return copy.deepcopy(self.draft)
        if method == "DELETE":
            asset_id = int(path.rsplit("/", 1)[1])
            self.draft["assets"] = [a for a in self.draft["assets"] if a["id"] != asset_id]
            return None
        raise AssertionError((method, path))

    def upload_asset(self, release_id, path):
        self.calls.append(("upload_asset", release_id, path.name))
        if self.fail_upload:
            raise publisher.ReleaseError("GitHub upload failed")
        self.draft["assets"].append({"id": 100 + len(self.calls), "name": path.name, "size": path.stat().st_size,
            "digest": "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()})
        if self.corrupt_upload:
            self.draft["assets"][0]["digest"] = "sha256:" + "0" * 64
        return ""


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.artifacts = self.root / "artifacts"
        self.artifacts.mkdir()
        self.cargo = self.root / "Cargo.toml"
        self.cargo.write_text('[package]\nversion = "1.2.3"\n')
        for name in NAMES:
            (self.artifacts / name).write_bytes(b"abc")
        self.github = FakeGitHub(self.artifacts)

    def publish(self, tag="v1.2.3"):
        return publisher.publish(self.github, self.artifacts, self.cargo, tag, SHA)

    def test_generates_checksums_for_exact_installer_set(self):
        result = publisher.prepare_assets(self.artifacts, self.cargo, "v1.2.3")
        self.assertEqual(set(result), {*NAMES, "SHA256SUMS.txt"})
        self.assertEqual((self.artifacts / "SHA256SUMS.txt").read_text(),
                         "".join(f"{DIGEST}  {name}\n" for name in sorted(NAMES)))

    def test_missing_package_prevents_every_github_call(self):
        (self.artifacts / NAMES[0]).unlink()
        with self.assertRaisesRegex(publisher.ReleaseError, "Missing release packages"):
            self.publish()
        self.assertEqual(self.github.calls, [])

    def test_tag_version_mismatch_prevents_every_github_call(self):
        with self.assertRaisesRegex(publisher.ReleaseError, "must match"):
            self.publish("v9.9.9")
        self.assertEqual(self.github.calls, [])

    def test_unexpected_file_and_symlink_are_rejected(self):
        extra = self.artifacts / "private.env"
        extra.write_text("fixture")
        with self.assertRaisesRegex(publisher.ReleaseError, "Unexpected release files"):
            self.publish()
        extra.unlink()
        package = self.artifacts / NAMES[0]
        package.unlink()
        package.symlink_to(self.cargo)
        with self.assertRaisesRegex(publisher.ReleaseError, "Invalid release package"):
            self.publish()
        self.assertEqual(self.github.calls, [])

    def test_publishes_only_after_all_uploads_pass_digest_checks(self):
        self.github.annotated = True
        self.assertEqual(self.publish(), "https://github.com/fikrifirmanf/ferrum-ssh/releases/tag/v1.2.3")
        calls = self.github.calls
        create = next(c for c in calls if c[0] == "POST")
        self.assertTrue(create[2]["draft"])
        self.assertFalse(create[2]["prerelease"])
        self.assertEqual(create[2]["target_commitish"], SHA)
        self.assertEqual(calls[-1], ("PATCH", "repos/fikrifirmanf/ferrum-ssh/releases/7",
                         {"draft": False, "prerelease": False, "make_latest": "true"}))
        self.assertEqual({a["name"] for a in self.github.draft["assets"]}, {*NAMES, "SHA256SUMS.txt"})

    def test_changed_git_tag_is_rejected_before_draft_creation(self):
        self.github.tag_sha = "b" * 40
        with self.assertRaisesRegex(publisher.ReleaseError, "does not point"):
            self.publish()
        self.assertFalse(any(call[0] in ("POST", "PATCH", "upload_asset") for call in self.github.calls))

    def test_failed_upload_leaves_release_as_draft(self):
        self.github.fail_upload = True
        with self.assertRaisesRegex(publisher.ReleaseError, "upload failed"):
            self.publish()
        self.assertTrue(self.github.draft["draft"])
        self.assertFalse(any(call[0] == "PATCH" for call in self.github.calls))

    def test_tampered_upload_never_becomes_public(self):
        self.github.corrupt_upload = True
        with self.assertRaisesRegex(publisher.ReleaseError, "checksum verification failed"):
            self.publish()
        self.assertTrue(self.github.draft["draft"])
        self.assertFalse(any(call[0] == "PATCH" for call in self.github.calls))

    def test_matching_published_release_is_idempotent(self):
        self.publish()
        self.github.existing = copy.deepcopy(self.github.draft)
        self.github.calls = []
        self.publish()
        self.assertFalse(any(call[0] in ("POST", "PATCH", "upload_asset") for call in self.github.calls))

    def test_published_release_with_different_bytes_is_never_overwritten(self):
        self.publish()
        self.github.existing = copy.deepcopy(self.github.draft)
        self.github.calls = []
        (self.artifacts / NAMES[0]).write_bytes(b"rebuilt installer")
        with self.assertRaises(publisher.ReleaseError):
            self.publish()
        self.assertFalse(any(call[0] in ("POST", "PATCH", "upload_asset") for call in self.github.calls))

    def test_draft_from_failed_run_is_cleared_and_reused(self):
        self.github.fail_upload = True
        with self.assertRaises(publisher.ReleaseError):
            self.publish()
        self.github.fail_upload = False
        self.github.draft["assets"] = [{"id": 5, "name": NAMES[0], "size": 1, "digest": "sha256:" + "0" * 64}]
        self.github.existing = copy.deepcopy(self.github.draft)
        self.github.calls = []
        self.publish()
        self.assertFalse(any(call[0] == "POST" for call in self.github.calls))
        self.assertIn(("DELETE", "repos/fikrifirmanf/ferrum-ssh/releases/assets/5", None), self.github.calls)
        self.assertFalse(self.github.draft["draft"])
        self.assertEqual(sorted(a["name"] for a in self.github.draft["assets"]), sorted([*NAMES, "SHA256SUMS.txt"]))

    def test_prerelease_tag_is_not_marked_latest(self):
        self.cargo.write_text('[package]\nversion = "1.2.3-beta.1"\n')
        for name in NAMES:
            (self.artifacts / name).rename(self.artifacts / name.replace("1.2.3", "1.2.3-beta.1"))
        self.publish("v1.2.3-beta.1")
        self.assertEqual(self.github.calls[-1][2], {"draft": False, "prerelease": True, "make_latest": "false"})


class GitHubClientTests(unittest.TestCase):
    def test_release_lookup_finds_draft_hidden_from_tag_endpoint(self):
        draft = {"id": 7, "tag_name": "v1.2.3", "draft": True}
        responses = {
            "repos/fikrifirmanf/ferrum-ssh/releases/tags/v1.2.3": publisher.ReleaseError("gh: Not Found (HTTP 404)"),
            "repos/fikrifirmanf/ferrum-ssh/releases?per_page=100": [
                {"id": 6, "tag_name": "v1.2.2", "draft": True}, draft],
        }

        def api(method, path, payload=None):
            response = responses[path]
            if isinstance(response, Exception):
                raise response
            return response

        github = publisher.GitHub()
        with mock.patch.object(github, "api", side_effect=api):
            self.assertEqual(github.release("v1.2.3"), draft)

    def test_release_lookup_reraises_errors_other_than_not_found(self):
        github = publisher.GitHub()
        with mock.patch.object(github, "api", side_effect=publisher.ReleaseError("HTTP 401")):
            with self.assertRaisesRegex(publisher.ReleaseError, "401"):
                github.release("v1.2.3")

    def test_assets_upload_to_uploads_host(self):
        with tempfile.NamedTemporaryFile() as asset, mock.patch.object(publisher.subprocess, "run") as run:
            publisher.GitHub().upload_asset(7, Path(asset.name))
        endpoint = run.call_args.args[0][4]
        self.assertTrue(endpoint.startswith("https://uploads.github.com/repos/fikrifirmanf/ferrum-ssh/releases/7/assets?name="),
                        endpoint)


if __name__ == "__main__":
    unittest.main()
