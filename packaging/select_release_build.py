"""Select immutable native artifacts from one successful exact-source main build."""

from datetime import datetime, timezone
import os
from pathlib import Path
import re
from urllib.parse import urlencode

from build_archive import read_release_version
from prepare_release import TARGETS
from publish_release import GitHub, ReleaseError


def positive_id(value):
    return type(value) is int and value > 0


def select_build(read, repository, commit, tag, source_version, now=None):
    """Discovery only: downloaded archives/smoke still require prepare_release.py."""
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository):
        raise ValueError("Require owner/repo")
    if not re.fullmatch(r"[0-9a-f]{40}", commit) or tag != "v" + source_version:
        raise ValueError("Require the exact source SHA and VERSION tag")
    now = now or datetime.now(timezone.utc)
    base = f"/repos/{repository}"
    query = urlencode({"event": "push", "branch": "main", "head_sha": commit,
                       "status": "success", "per_page": 20})
    runs = read(f"{base}/actions/workflows/build.yml/runs?{query}")["workflow_runs"]
    packages = [f"recordbox-{platform}-{arch}-{commit}" for platform, arch in TARGETS]
    smoke = [f"smoke-{platform}-{arch}-{commit}" for platform, arch in TARGETS]
    required = set(packages + smoke + [f"release-bundle-{commit}"])
    for run in runs:
        if not (positive_id(run.get("id")) and run.get("event") == "push"
                and run.get("head_branch") == "main" and run.get("head_sha") == commit
                and run.get("status") == "completed" and run.get("conclusion") == "success"
                and run.get("path") == ".github/workflows/build.yml"
                and run.get("head_repository", {}).get("full_name") == repository
                and positive_id(run["head_repository"].get("id"))):
            continue
        response = read(f"{base}/actions/runs/{run['id']}/artifacts?per_page=100")
        rows = response["artifacts"]
        # A truncated listing cannot establish a complete, unambiguous set.
        if response.get("total_count") != len(rows):
            continue
        selected = {}
        valid = True
        for row in rows:
            name = row.get("name")
            if name not in required:
                continue
            provenance = row.get("workflow_run", {})
            try:
                expires = datetime.fromisoformat(row["expires_at"].replace("Z", "+00:00"))
                available = expires > now
            except (KeyError, TypeError, ValueError):
                available = False
            if not (name not in selected and positive_id(row.get("id"))
                    and row.get("expired") is False and available
                    and type(row.get("size_in_bytes")) is int and row["size_in_bytes"] > 0
                    and provenance.get("id") == run["id"]
                    and provenance.get("head_sha") == commit and provenance.get("head_branch") == "main"
                    and provenance.get("repository_id") == run["head_repository"]["id"]
                    and provenance.get("head_repository_id") == run["head_repository"]["id"]):
                valid = False
                break
            selected[name] = row["id"]
        if valid and set(selected) == required and len(set(selected.values())) == len(required):
            return {"reuse_run_id": str(run["id"]),
                    "package_ids": ",".join(str(selected[name]) for name in packages),
                    "smoke_ids": ",".join(str(selected[name]) for name in smoke)}
    return None


def main():
    selection = None
    # PR, manual and ordinary branch builds always exercise every native target.
    if os.environ.get("GITHUB_EVENT_NAME") == "push" and os.environ.get("GITHUB_REF", "").startswith("refs/tags/v"):
        try:
            source = Path(__file__).resolve().parents[1]
            github = GitHub(os.environ["GH_TOKEN"])
            selection = select_build(lambda path: github.api("GET", path), os.environ["GITHUB_REPOSITORY"],
                                     os.environ["GITHUB_SHA"], os.environ["GITHUB_REF_NAME"],
                                     read_release_version(source))
        except (ReleaseError, OSError, ValueError, KeyError, TypeError, AttributeError) as error:
            # Discovery is optional. Failure after selection/download must NOT fall back.
            print(f"No reusable main build selected ({type(error).__name__}); building all native targets.")
    outputs = selection or {"reuse_run_id": "", "package_ids": "", "smoke_ids": ""}
    with Path(os.environ["GITHUB_OUTPUT"]).open("a", encoding="utf-8") as output:
        for key, value in outputs.items():
            output.write(f"{key}={value}\n")
    print(f"Reusing exact-source main build {selection['reuse_run_id']}" if selection else "Building all native targets")


if __name__ == "__main__":
    main()
