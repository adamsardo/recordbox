"""Fail-closed provenance selection and the workflow's actual aggregate gate."""

import copy
from datetime import datetime, timezone
import os
from pathlib import Path
import re
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import select_release_build as selector
from publish_release import ReleaseError


COMMIT = "a" * 40
REPOSITORY = "example/recordbox"
NOW = datetime(2026, 11, 19, tzinfo=timezone.utc)


class SelectionTests(unittest.TestCase):
    def setUp(self):
        self.run = {"id": 123, "event": "push", "head_branch": "main", "head_sha": COMMIT,
                    "status": "completed", "conclusion": "success", "path": ".github/workflows/build.yml",
                    "head_repository": {"full_name": REPOSITORY, "id": 10}}
        names = [f"{kind}-{platform}-{arch}-{COMMIT}" for kind in ("recordbox", "smoke")
                 for platform, arch in selector.TARGETS] + [f"release-bundle-{COMMIT}"]
        self.artifacts = [{"id": index, "name": name, "expired": False, "size_in_bytes": 50,
                           "expires_at": "2026-12-01T00:00:00Z",
                           "workflow_run": {"id": 123, "head_sha": COMMIT, "head_branch": "main",
                                            "repository_id": 10, "head_repository_id": 10}}
                          for index, name in enumerate(names, 1)]
        self.runs = [self.run]
        self.listings = {123: self.artifacts}
        self.calls = []

    def read(self, path):
        self.calls.append(path)
        if "/workflows/" in path:
            return {"workflow_runs": self.runs}
        run_id = int(re.search(r"/runs/(\d+)/", path)[1])
        rows = self.listings[run_id]
        return {"total_count": len(rows), "artifacts": rows}

    def select(self, **kwargs):
        return selector.select_build(self.read, REPOSITORY, COMMIT, kwargs.get("tag", "v0.5.1"),
                                     "0.5.1", now=NOW)

    def test_complete_exact_main_run_selects_immutable_ids_not_the_release_bundle(self):
        self.assertEqual(self.select(), {"reuse_run_id": "123", "package_ids": "1,2,3,4", "smoke_ids": "5,6,7,8"})
        self.assertIn("event=push&branch=main&head_sha=" + COMMIT, self.calls[0])

    def test_wrong_version_is_rejected_before_any_api_call(self):
        with self.assertRaises(ValueError):
            self.select(tag="v0.5.2")
        self.assertEqual(self.calls, [])

    def test_untrusted_or_unsuccessful_runs_never_fetch_artifacts(self):
        changes = {"event": "pull_request", "head_branch": "feature", "head_sha": "b" * 40,
                   "status": "in_progress", "conclusion": "failure", "path": ".github/workflows/other.yml",
                   "head_repository": {"full_name": "fork/recordbox"}, "id": True}
        for key, value in changes.items():
            with self.subTest(key=key):
                self.runs = [{**self.run, key: value}]
                self.calls.clear()
                self.assertIsNone(self.select())
                self.assertEqual(len(self.calls), 1)

    def test_every_native_package_smoke_and_prior_validation_bundle_is_required(self):
        for index in range(len(self.artifacts)):
            with self.subTest(missing=self.artifacts[index]["name"]):
                self.listings[123] = self.artifacts[:index] + self.artifacts[index + 1:]
                self.assertIsNone(self.select())

    def test_expired_ambiguous_or_foreign_artifacts_are_rejected(self):
        changes = [{"expired": True}, {"expired": None}, {"expires_at": "2026-11-18T00:00:00Z"},
                   {"expires_at": "2026-11-19T00:00:00Z"}, {"expires_at": "invalid"},
                   {"expires_at": "2026-12-01"}, {"size_in_bytes": 0}, {"id": True},
                   {"id": self.artifacts[1]["id"]}, {"workflow_run": {"id": 999}},
                   {"workflow_run": {**self.artifacts[0]["workflow_run"], "head_sha": "b" * 40}},
                   {"workflow_run": {**self.artifacts[0]["workflow_run"], "head_branch": "feature"}},
                   {"workflow_run": {**self.artifacts[0]["workflow_run"], "repository_id": 99, "head_repository_id": 99}},
                   {"workflow_run": {**self.artifacts[0]["workflow_run"], "head_repository_id": 99}}]
        for change in changes:
            with self.subTest(change=change):
                rows = copy.deepcopy(self.artifacts)
                rows[0].update(change)
                self.listings[123] = rows
                self.assertIsNone(self.select())
        self.listings[123] = self.artifacts + [copy.deepcopy(self.artifacts[0])]
        self.assertIsNone(self.select())

    def test_partial_sets_from_separate_successful_runs_are_never_combined(self):
        self.runs.append({**self.run, "id": 124})
        self.listings = {123: self.artifacts[:4], 124: self.artifacts[4:]}
        self.assertIsNone(self.select())

    def test_later_complete_run_can_follow_an_unavailable_run(self):
        self.runs.insert(0, {**self.run, "id": 122})
        self.listings[122] = []
        self.assertEqual(self.select()["reuse_run_id"], "123")

    def test_truncated_listing_uses_native_fallback(self):
        def read(path):
            result = self.read(path)
            if "artifacts" in result:
                result["total_count"] += 100
            return result
        self.assertIsNone(selector.select_build(read, REPOSITORY, COMMIT, "v0.5.1", "0.5.1", NOW))

    def test_cli_keeps_pr_branch_and_manual_builds_native_and_api_errors_fall_back(self):
        for event, ref in (("pull_request", "refs/pull/1/merge"), ("push", "refs/heads/main"),
                           ("workflow_dispatch", "refs/tags/v0.5.1"), ("push", "refs/tags/v0.5.1")):
            with self.subTest(event=event, ref=ref), tempfile.TemporaryDirectory() as temporary:
                output = Path(temporary) / "outputs"
                environment = {"GITHUB_EVENT_NAME": event, "GITHUB_REF": ref, "GITHUB_REF_NAME": "v0.5.1",
                               "GITHUB_SHA": COMMIT, "GITHUB_REPOSITORY": REPOSITORY,
                               "GH_TOKEN": "not-a-real-token", "GITHUB_OUTPUT": str(output)}
                with patch.dict(os.environ, environment, clear=True), patch.object(selector, "GitHub") as github:
                    github.return_value.api.side_effect = ReleaseError("unavailable")
                    selector.main()
                    self.assertEqual(github.call_count, int(event == "push" and ref.startswith("refs/tags/v")))
                self.assertEqual(output.read_text(), "reuse_run_id=\npackage_ids=\nsmoke_ids=\n")


class WorkflowGateTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.workflow = (Path(__file__).resolve().parents[1] / ".github/workflows/build.yml").read_text()
        required = cls.workflow.split("  required:\n", 1)[1].split("  publish:\n", 1)[0]
        cls.gate = "\n".join(line[10:] for line in required.split("        run: |\n", 1)[1].splitlines() if line.strip())

    def check_gate(self, **changes):
        environment = {**os.environ, "VERSION_CHECK": "success", "TESTS": "success", "AUDIT": "success",
                       "RELEASE_ASSETS": "success", "PACKAGES": "success", "REUSE_RUN_ID": "",
                       "GITHUB_EVENT_NAME": "push", "GITHUB_REF": "refs/tags/v0.5.1", **changes}
        return subprocess.run(["bash", "-e", "-c", self.gate], env=environment, capture_output=True).returncode

    def test_both_planned_paths_require_all_other_checks_and_fresh_asset_verification(self):
        for mode in ({}, {"REUSE_RUN_ID": "123", "PACKAGES": "skipped"}):
            self.assertEqual(self.check_gate(**mode), 0)
            for job in ("VERSION_CHECK", "TESTS", "AUDIT", "RELEASE_ASSETS"):
                for failure in ("failure", "cancelled", "skipped", ""):
                    with self.subTest(mode=mode, job=job, failure=failure):
                        self.assertNotEqual(self.check_gate(**{**mode, job: failure}), 0)

    def test_no_blanket_skipped_acceptance_or_unplanned_reuse(self):
        for values in ({"PACKAGES": "skipped"}, {"PACKAGES": "failure"}, {"PACKAGES": "cancelled"},
                       {"REUSE_RUN_ID": "123"}, {"REUSE_RUN_ID": "invalid", "PACKAGES": "skipped"},
                       {"REUSE_RUN_ID": "123", "PACKAGES": "skipped", "GITHUB_EVENT_NAME": "pull_request"},
                       {"REUSE_RUN_ID": "123", "PACKAGES": "skipped", "GITHUB_REF": "refs/heads/main"}):
            with self.subTest(values=values):
                self.assertNotEqual(self.check_gate(**values), 0)

    def test_reuse_changes_only_native_execution_and_downloads_exact_ids_from_one_run(self):
        for job in ("tests", "audit"):
            body = re.split(r"(?m)^  [a-z_]+:\n", self.workflow.split(f"  {job}:\n", 1)[1], maxsplit=1)[0]
            self.assertIn("    steps:\n", body)
            self.assertNotIn("reuse_run_id", body)
        self.assertEqual(self.workflow.count("run-id: ${{ needs.version.outputs.reuse_run_id }}"), 2)
        self.assertIn("artifact-ids: ${{ needs.version.outputs.package_ids }}", self.workflow)
        self.assertIn("artifact-ids: ${{ needs.version.outputs.smoke_ids }}", self.workflow)
        self.assertIn("python packaging/prepare_release.py --input release-input", self.workflow)
        self.assertNotIn("continue-on-error:", self.workflow)


if __name__ == "__main__":
    unittest.main()
