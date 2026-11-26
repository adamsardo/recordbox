"""Release publishing tests use a fake GitHub service and never send requests."""

import copy
import hashlib
import io
from pathlib import Path
import tempfile
import unittest
import urllib.error
import urllib.parse

import publish_release as publisher


SOURCE = "a" * 40
BASE = "/repos/fantuz/recordbox"
TAG = "v0.5.0"


class FakeGitHub:
    def __init__(self, directory):
        self.directory = directory
        self.release = None
        self.assets = []
        self.mutations = []
        self.tag = SOURCE
        self.digest_missing = False
        self.upload_error = False
        self.retarget = False
        self.omit_last = False
        self.tag_endpoint_error = 404
        self.downloads = []
        self.annotated = False
        self.latest = None
        self.latest_error = 404

    def api(self, method, path, body=None):
        if method == "GET" and path == f"{BASE}/releases/latest":
            if self.latest is None:
                raise publisher.APIError(self.latest_error)
            return copy.deepcopy(self.latest)
        if method == "GET" and path == f"{BASE}/git/ref/tags/{TAG}":
            return {"object": {"type": "tag" if self.annotated else "commit", "sha": self.tag}}
        if method == "GET" and path == f"{BASE}/git/tags/{SOURCE}":
            return {"object": {"type": "commit", "sha": SOURCE}}
        if method == "GET" and path == f"{BASE}/releases/tags/{TAG}":
            if self.release and not self.release["draft"]:
                return copy.deepcopy(self.release)
            raise publisher.APIError(self.tag_endpoint_error)
        if method == "GET" and path == f"{BASE}/releases?per_page=100&page=1":
            return [copy.deepcopy(self.release)] if self.release else []
        if method == "POST" and path == f"{BASE}/releases":
            self.mutations.append((method, copy.deepcopy(body)))
            self.release = {**body, "id": 42, "html_url": "https://github.com/fantuz/recordbox/releases/tag/v0.5.0",
                            "upload_url": f"https://uploads.github.com{BASE}/releases/42/assets{{?name,label}}"}
            return copy.deepcopy(self.release)
        if method == "GET" and path == f"{BASE}/releases/42":
            return copy.deepcopy(self.release)
        if method == "GET" and path == f"{BASE}/releases/42/assets?per_page=100":
            assets = self.assets[:-1] if self.omit_last and len(self.assets) == 6 else self.assets
            return copy.deepcopy(assets)
        if method == "PATCH" and path == f"{BASE}/releases/42":
            self.mutations.append((method, copy.deepcopy(body)))
            self.release.update(body)
            return copy.deepcopy(self.release)
        raise AssertionError((method, path, body))

    def upload(self, url, path):
        self.mutations.append(("upload", path.name))
        if self.upload_error:
            raise publisher.APIError(502)
        assert self.release["draft"]
        assert urllib.parse.parse_qs(urllib.parse.urlsplit(url).query)["name"] == [path.name]
        self.assets.append({"id": len(self.assets) + 100, "name": path.name,
                            "size": path.stat().st_size, "state": "uploaded",
                            "digest": None if self.digest_missing else f"sha256:{publisher.file_digest(path)}"})
        if self.retarget:
            self.tag = "b" * 40

    def download_digest(self, path):
        self.downloads.append(path)
        asset = next(asset for asset in self.assets if str(asset["id"]) == path.rsplit("/", 1)[1])
        local = self.directory / "assets" / asset["name"]
        return local.stat().st_size, publisher.file_digest(local)


class PublishTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        assets = self.directory / "assets"
        assets.mkdir()
        names = [f"Recordbox-0.5.0-{platform}.{extension}" for platform, extension in (
            ("macos-arm64", "zip"), ("macos-x86_64", "zip"),
            ("windows-x86_64", "zip"), ("linux-x86_64", "tar.gz"))]
        for name in names:
            (assets / name).write_bytes(name.encode())
        (assets / "COMMIT-LOG.txt").write_text(f"commit {SOURCE}\nRelease source\n")
        (assets / "SHA256SUMS").write_text("".join(
            f"{publisher.file_digest(path)}  {path.name}\n" for path in sorted(assets.iterdir())))
        self.body = f"Native release.\n<!-- recordbox-release-source: {SOURCE} -->\n"
        (self.directory / "body.md").write_text(self.body)
        self.github = FakeGitHub(self.directory)

    def publish(self):
        return publisher.publish_release("fantuz/recordbox", TAG, SOURCE, self.directory, self.github)

    def prepare_draft(self):
        self.github.release = self.github.api("POST", f"{BASE}/releases", {
            "tag_name": TAG, "target_commitish": SOURCE, "name": TAG,
            "body": self.body, "draft": True, "prerelease": False})
        self.github.mutations.clear()

    def assert_draft_untouched(self):
        self.assertTrue(self.github.release["draft"])
        self.assertFalse(any(method == "PATCH" for method, _ in self.github.mutations))

    def test_first_release_uploads_six_assets_then_marks_latest(self):
        result = self.publish()
        self.assertFalse(result["draft"])
        self.assertEqual(len(self.github.assets), 6)
        self.assertEqual(self.github.mutations[-1], ("PATCH", {"draft": False, "make_latest": "true"}))
        self.assertTrue(self.github.mutations[0][1]["draft"])
        self.assertEqual(self.github.mutations[0][1]["target_commitish"], SOURCE)

    def test_newer_stable_release_becomes_latest(self):
        self.github.latest = {"tag_name": "v0.4.9", "draft": False, "prerelease": False}
        self.publish()
        self.assertEqual(self.github.mutations[-1][1]["make_latest"], "true")

    def test_older_stable_release_does_not_downgrade_latest(self):
        self.github.latest = {"tag_name": "v0.10.0", "draft": False, "prerelease": False}
        self.publish()
        self.assertEqual(self.github.mutations[-1][1]["make_latest"], "false")

    def test_equal_latest_version_is_rejected(self):
        self.github.latest = {"tag_name": TAG, "draft": False, "prerelease": False}
        with self.assertRaisesRegex(publisher.ReleaseError, "already published"):
            self.publish()
        self.assert_draft_untouched()

    def test_unknown_or_nonstable_latest_is_rejected(self):
        for tag in (None, "release", "v0.6.0-rc.1", "v00.6.0"):
            with self.subTest(tag=tag):
                self.github.latest = {"tag_name": tag, "draft": False, "prerelease": False}
                with self.assertRaisesRegex(publisher.ReleaseError, "stable vMAJOR"):
                    self.publish()
                self.assert_draft_untouched()

    def test_latest_permission_error_does_not_publish(self):
        self.github.latest_error = 403
        with self.assertRaises(publisher.APIError) as error:
            self.publish()
        self.assertEqual(error.exception.status, 403)
        self.assert_draft_untouched()

    def test_latest_published_during_upload_is_respected(self):
        self.github.latest = {"tag_name": "v0.4.0", "draft": False, "prerelease": False}
        original = self.github.upload

        def newer_release_during_upload(url, path):
            original(url, path)
            self.github.latest = {"tag_name": "v0.6.0", "draft": False, "prerelease": False}

        self.github.upload = newer_release_during_upload
        self.publish()
        self.assertEqual(self.github.mutations[-1][1]["make_latest"], "false")

    def test_missing_remote_digests_download_every_asset(self):
        self.github.digest_missing = True
        self.publish()
        self.assertEqual(len(self.github.downloads), 6)

    def test_draft_resume_skips_only_verified_existing_assets(self):
        self.prepare_draft()
        path = self.directory / "assets" / "COMMIT-LOG.txt"
        self.github.upload("https://uploads.github.com/?name=COMMIT-LOG.txt", path)
        self.github.mutations.clear()
        self.publish()
        self.assertEqual(sum(method == "upload" for method, _ in self.github.mutations), 5)
        self.assertFalse(any(method == "POST" for method, _ in self.github.mutations))

    def test_remote_digest_mismatch_blocks_resume(self):
        self.prepare_draft()
        path = self.directory / "assets" / "COMMIT-LOG.txt"
        self.github.upload("https://uploads.github.com/?name=COMMIT-LOG.txt", path)
        self.github.assets[0]["digest"] = "sha256:" + "0" * 64
        self.github.mutations.clear()
        with self.assertRaisesRegex(publisher.ReleaseError, "digest mismatch"):
            self.publish()
        self.assertEqual(self.github.mutations, [])

    def test_downloaded_digest_mismatch_blocks_publication(self):
        self.github.digest_missing = True
        self.github.download_digest = lambda path: (1, "0" * 64)
        with self.assertRaisesRegex(publisher.ReleaseError, "digest mismatch"):
            self.publish()
        self.assert_draft_untouched()

    def test_missing_remote_asset_blocks_publication(self):
        self.github.omit_last = True
        with self.assertRaisesRegex(publisher.ReleaseError, "missing, extra"):
            self.publish()
        self.assert_draft_untouched()

    def test_extra_remote_asset_blocks_any_upload(self):
        self.prepare_draft()
        self.github.assets.append({"name": "unrelated.zip"})
        with self.assertRaisesRegex(publisher.ReleaseError, "missing, extra"):
            self.publish()
        self.assertEqual(self.github.mutations, [])

    def test_changed_tag_blocks_publication(self):
        self.github.retarget = True
        with self.assertRaisesRegex(publisher.ReleaseError, "Tag does not resolve"):
            self.publish()
        self.assert_draft_untouched()

    def test_annotated_tag_is_peeled(self):
        self.github.annotated = True
        self.assertFalse(self.publish()["draft"])

    def test_rerun_verifies_public_assets_without_writes(self):
        self.github.digest_missing = True
        published = self.publish()
        self.github.mutations.clear()
        self.github.downloads.clear()
        self.github.latest = {"tag_name": "v0.6.0", "draft": False, "prerelease": False}
        self.assertEqual(self.publish(), published)
        self.assertEqual(len(self.github.downloads), len(self.github.assets))
        self.assertEqual(self.github.mutations, [])

    def test_public_release_with_missing_or_changed_assets_fails_without_writes(self):
        self.publish()
        self.github.mutations.clear()
        self.github.assets[0]["digest"] = "sha256:" + "0" * 64
        with self.assertRaisesRegex(publisher.ReleaseError, "digest mismatch"):
            self.publish()
        self.github.assets.pop(0)
        with self.assertRaisesRegex(publisher.ReleaseError, "missing, extra"):
            self.publish()
        self.assertEqual(self.github.mutations, [])

    def test_public_release_with_different_metadata_fails_without_writes(self):
        published = self.publish()
        self.github.mutations.clear()
        for field, value in (("target_commitish", "b" * 40), ("body", "Edited notes"),
                             ("prerelease", True), ("name", "Another release")):
            with self.subTest(field=field):
                self.github.release = {**published, field: value}
                with self.assertRaisesRegex(publisher.ReleaseError, "does not belong"):
                    self.publish()
                self.assertEqual(self.github.mutations, [])

    def test_retarget_during_public_asset_verification_fails_without_writes(self):
        self.github.digest_missing = True
        self.publish()
        self.github.mutations.clear()
        download_digest = self.github.download_digest

        def move_tag_during_download(path):
            self.github.tag = "b" * 40
            return download_digest(path)

        self.github.download_digest = move_tag_during_download
        with self.assertRaisesRegex(publisher.ReleaseError, "Tag does not resolve"):
            self.publish()
        self.assertEqual(self.github.mutations, [])

    def test_wrong_source_draft_is_rejected(self):
        self.prepare_draft()
        self.github.release["target_commitish"] = "b" * 40
        with self.assertRaisesRegex(publisher.ReleaseError, "does not belong"):
            self.publish()
        self.assertEqual(self.github.mutations, [])

    def test_unowned_draft_body_is_rejected(self):
        self.prepare_draft()
        self.github.release["body"] = "Unrelated draft"
        with self.assertRaisesRegex(publisher.ReleaseError, "does not belong"):
            self.publish()
        self.assertEqual(self.github.mutations, [])

    def test_release_published_during_download_is_not_patched(self):
        self.github.digest_missing = True
        original = self.github.download_digest

        def concurrent_publication(path):
            self.github.release["draft"] = False
            return original(path)

        self.github.download_digest = concurrent_publication
        with self.assertRaisesRegex(publisher.ReleaseError, "publication state changed"):
            self.publish()
        self.assertFalse(any(method == "PATCH" for method, _ in self.github.mutations))

    def test_upload_failure_keeps_draft(self):
        self.github.upload_error = True
        with self.assertRaises(publisher.APIError):
            self.publish()
        self.assert_draft_untouched()

    def test_permission_error_does_not_create_release(self):
        self.github.tag_endpoint_error = 403
        with self.assertRaises(publisher.APIError):
            self.publish()
        self.assertEqual(self.github.mutations, [])

    def test_unsafe_upload_host_is_rejected(self):
        self.prepare_draft()
        self.github.release["upload_url"] = f"https://evil.example{BASE}/releases/42/assets{{?name,label}}"
        with self.assertRaisesRegex(publisher.ReleaseError, "Unsafe"):
            self.publish()
        self.assertEqual(self.github.mutations, [])

    def test_local_checksum_or_extra_file_is_rejected_before_api(self):
        (self.directory / "assets" / "COMMIT-LOG.txt").write_text("changed")
        with self.assertRaisesRegex(publisher.ReleaseError, "SHA256SUMS mismatch"):
            self.publish()
        self.assertEqual(self.github.mutations, [])
        (self.directory / "assets" / "extra.zip").write_bytes(b"extra")
        with self.assertRaisesRegex(publisher.ReleaseError, "every expected archive"):
            self.publish()

    def test_duplicate_checksum_entry_is_rejected(self):
        path = self.directory / "assets" / "SHA256SUMS"
        with path.open("a") as stream:
            stream.write(path.read_text().splitlines()[0] + "\n")
        with self.assertRaisesRegex(publisher.ReleaseError, "duplicate SHA256SUMS"):
            self.publish()
        self.assertEqual(self.github.mutations, [])

    def test_symlink_asset_is_rejected(self):
        path = self.directory / "assets" / "COMMIT-LOG.txt"
        target = self.directory / "commit-log.txt"
        path.rename(target)
        path.symlink_to(target)
        with self.assertRaisesRegex(publisher.ReleaseError, "without symlinks"):
            self.publish()
        self.assertEqual(self.github.mutations, [])


class HTTPTests(unittest.TestCase):
    def test_credentialed_upload_does_not_follow_redirect(self):
        redirect = publisher.NoRedirect()
        self.assertIsNone(redirect.redirect_request(None, None, 302, "Found", {}, "https://evil.example"))

    def test_download_redirect_drops_authorization(self):
        github = publisher.GitHub("secret-token")
        calls = []

        class Opener:
            def open(self, request, timeout):
                calls.append(request)
                if len(calls) == 1:
                    raise urllib.error.HTTPError(request.full_url, 302, "Found",
                        {"Location": "https://release-assets.githubusercontent.com/file"}, io.BytesIO())
                return io.BytesIO(b"archive")

        github.opener = Opener()
        self.assertEqual(github.download_digest(f"{BASE}/releases/assets/1"),
                         (7, hashlib.sha256(b"archive").hexdigest()))
        self.assertEqual(calls[0].get_header("Authorization"), "Bearer secret-token")
        self.assertIsNone(calls[1].get_header("Authorization"))

    def test_untrusted_download_redirect_is_rejected(self):
        github = publisher.GitHub("secret-token")
        github._open = lambda *args, **kwargs: (_ for _ in ()).throw(
            publisher.APIError(302, "https://evil.example/archive"))
        with self.assertRaisesRegex(publisher.ReleaseError, "Unsafe"):
            github.download_digest(f"{BASE}/releases/assets/1")


if __name__ == "__main__":
    unittest.main()
