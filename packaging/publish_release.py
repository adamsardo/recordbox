"""Publish a verified native release or verify an existing publication."""

import argparse
import hashlib
import http.client
import json
import os
from pathlib import Path
import re
import sys
import urllib.error
import urllib.parse
import urllib.request
from build_archive import release_archives


class ReleaseError(Exception):
    pass


class APIError(ReleaseError):
    def __init__(self, status, location=None):
        self.status, self.location = status, location
        super().__init__(f"GitHub API returned HTTP {status}")


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, message, headers, newurl):
        return None


class GitHub:
    def __init__(self, token):
        self.token = token
        self.opener = urllib.request.build_opener(NoRedirect())

    def _open(self, method, url, data=None, headers=None, authenticated=True):
        headers = {"User-Agent": "recordbox-release", "Accept": "application/vnd.github+json",
                   "X-GitHub-Api-Version": "2022-11-28", **(headers or {})}
        if authenticated:
            headers["Authorization"] = f"Bearer {self.token}"
        try:
            return self.opener.open(urllib.request.Request(url, data=data, headers=headers,
                                                         method=method), timeout=120)
        except urllib.error.HTTPError as exc:
            location = exc.headers.get("Location")
            exc.close()
            raise APIError(exc.code, location) from None
        except (urllib.error.URLError, OSError):
            raise ReleaseError("GitHub request failed (network error)") from None

    def api(self, method, path, body=None):
        data = json.dumps(body).encode() if body is not None else None
        with self._open(method, "https://api.github.com" + path, data,
                        {"Content-Type": "application/json"}) as response:
            return json.load(response)

    def upload(self, url, path):
        with path.open("rb") as stream:
            with self._open("POST", url, stream,
                            {"Content-Type": "application/octet-stream",
                             "Content-Length": str(path.stat().st_size)}) as response:
                return json.load(response)

    def download_digest(self, path):
        url, authenticated = "https://api.github.com" + path, True
        for _ in range(6):
            try:
                with self._open("GET", url, headers={"Accept": "application/octet-stream"},
                                authenticated=authenticated) as response:
                    digest, size = hashlib.sha256(), 0
                    while chunk := response.read(1024 * 1024):
                        digest.update(chunk)
                        size += len(chunk)
                    return size, digest.hexdigest()
            except APIError as exc:
                if exc.status not in (301, 302, 303, 307, 308) or not exc.location:
                    raise
                url = urllib.parse.urljoin(url, exc.location)
                parsed = urllib.parse.urlsplit(url)
                host = parsed.hostname or ""
                if (parsed.scheme != "https" or parsed.username or parsed.password or
                        parsed.port is not None or parsed.fragment or
                        not (host in ("github.com", "api.github.com") or
                             host.endswith(".githubusercontent.com"))):
                    raise ReleaseError("Unsafe asset download redirect")
                authenticated = False
        raise ReleaseError("Too many asset download redirects")


def file_digest(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def stable_version(tag):
    if not isinstance(tag, str) or not re.fullmatch(r"v(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)", tag):
        raise ReleaseError("Tag must be a stable vMAJOR.MINOR.PATCH version")
    return tuple(map(int, tag[1:].split(".")))


def local_release(repository, tag, source, directory):
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository):
        raise ReleaseError("Repository must be owner/repo")
    stable_version(tag)
    if not re.fullmatch(r"[0-9a-f]{40}", source):
        raise ReleaseError("Source commit must be a full lowercase Git commit SHA")
    names = {name for _, _, _, name in release_archives(tag[1:])}
    subjects = names | {"COMMIT-LOG.txt"}
    assets = Path(directory) / "assets"
    if assets.is_symlink() or {p.name for p in assets.iterdir()} != subjects | {"SHA256SUMS"}:
        raise ReleaseError("Local assets must contain every expected archive and both manifests")
    files = {name: assets / name for name in subjects | {"SHA256SUMS"}}
    if any(path.is_symlink() or not path.is_file() or not path.stat().st_size for path in files.values()):
        raise ReleaseError("Assets must be nonempty regular files without symlinks")
    checksums = {}
    for line in files["SHA256SUMS"].read_text(encoding="utf-8").splitlines():
        match = re.fullmatch(r"([0-9a-f]{64})  ([^/\\]+)", line)
        if not match or match[2] in checksums:
            raise ReleaseError("Invalid or duplicate SHA256SUMS entry")
        checksums[match[2]] = match[1]
    if set(checksums) != subjects:
        raise ReleaseError("SHA256SUMS must cover every expected archive and COMMIT-LOG.txt")
    expected = {name: (path.stat().st_size, file_digest(path)) for name, path in files.items()}
    if any(expected[name][1] != digest for name, digest in checksums.items()):
        raise ReleaseError("Local SHA256SUMS mismatch")
    body = (Path(directory) / "body.md").read_text(encoding="utf-8")
    marker = f"<!-- recordbox-release-source: {source} -->"
    if body.count("recordbox-release-source:") != 1 or marker not in body:
        raise ReleaseError("Release body must identify the exact source commit")
    if source not in files["COMMIT-LOG.txt"].read_text(encoding="utf-8"):
        raise ReleaseError("Commit log must include the source commit")
    return files, expected, body


def publish_release(repository, tag, source, directory, github):
    files, expected, body = local_release(repository, tag, source, directory)
    base = f"/repos/{repository}"

    def check_tag():
        obj = github.api("GET", f"{base}/git/ref/tags/{tag}")["object"]
        for _ in range(8):
            if obj.get("type") == "commit" and obj.get("sha") == source:
                return
            if obj.get("type") != "tag" or not re.fullmatch(r"[0-9a-f]{40}", obj.get("sha", "")):
                break
            obj = github.api("GET", f"{base}/git/tags/{obj['sha']}")["object"]
        raise ReleaseError("Tag does not resolve to the expected source commit")

    def checked_release(release, *, draft):
        if release.get("draft") is not draft:
            raise ReleaseError("Release publication state changed during verification")
        if (release.get("tag_name") != tag or release.get("target_commitish") != source or
                release.get("body") != body or release.get("name") != tag or
                release.get("prerelease") is not False or type(release.get("id")) is not int or
                release["id"] <= 0):
            raise ReleaseError("Existing release does not belong to this prepared source release")
        return release

    def remote_assets(release_id, complete=False):
        assets = github.api("GET", f"{base}/releases/{release_id}/assets?per_page=100")
        names = [asset.get("name") for asset in assets]
        if len(set(names)) != len(names) or not set(names) <= files.keys() or (complete and set(names) != files.keys()):
            raise ReleaseError("Remote release has missing, extra, or duplicate assets")
        for asset in assets:
            size, digest = expected[asset["name"]]
            if asset.get("size") != size or asset.get("state") != "uploaded":
                raise ReleaseError(f"Remote asset is incomplete: {asset['name']}")
            remote_digest = asset.get("digest")
            if not remote_digest:
                asset_id = asset.get("id")
                if type(asset_id) is not int:
                    raise ReleaseError("Invalid remote asset ID")
                actual = github.download_digest(f"{base}/releases/assets/{asset_id}")
                if actual != (size, digest):
                    raise ReleaseError(f"Remote asset digest mismatch: {asset['name']}")
            elif remote_digest != f"sha256:{digest}":
                raise ReleaseError(f"Remote asset digest mismatch: {asset['name']}")
        return set(names)

    check_tag()
    try:
        release = github.api("GET", f"{base}/releases/tags/{tag}")
    except APIError as exc:
        if exc.status != 404:
            raise
        # The tag endpoint need not expose drafts. Find them before creating one.
        matches = []
        for page in range(1, 101):
            releases = github.api("GET", f"{base}/releases?per_page=100&page={page}")
            matches.extend(item for item in releases if item.get("tag_name") == tag)
            if len(releases) < 100:
                break
        else:
            raise ReleaseError("Too many releases to safely locate an existing draft")
        if len(matches) > 1:
            raise ReleaseError("Multiple releases already use this tag")
        release = matches[0] if matches else github.api("POST", f"{base}/releases", {
            "tag_name": tag, "target_commitish": source, "name": tag, "body": body,
            "draft": True, "prerelease": False, "make_latest": "false"})
    if release.get("draft") is False:
        checked_release(release, draft=False)
        release_id = release["id"]
        remote_assets(release_id, complete=True)
        release = checked_release(github.api("GET", f"{base}/releases/{release_id}"), draft=False)
        check_tag()
        return release

    checked_release(release, draft=True)
    release_id = release["id"]
    existing = remote_assets(release_id)
    for name in sorted(files.keys() - existing):
        release = checked_release(github.api("GET", f"{base}/releases/{release_id}"), draft=True)
        upload_url = release.get("upload_url", "").removesuffix("{?name,label}")
        if upload_url != f"https://uploads.github.com{base}/releases/{release_id}/assets":
            raise ReleaseError("Unsafe GitHub asset upload URL")
        github.upload(upload_url + "?name=" + urllib.parse.quote(name, safe=""), files[name])
    remote_assets(release_id, complete=True)
    # Read this after uploads: a newer release may have appeared while they ran.
    try:
        latest = github.api("GET", f"{base}/releases/latest")
    except APIError as exc:
        if exc.status != 404:
            raise
        make_latest = "true"
    else:
        if latest.get("draft") is not False or latest.get("prerelease") is not False:
            raise ReleaseError("Latest release is not a published stable release")
        latest_version = stable_version(latest.get("tag_name"))
        requested_version = stable_version(tag)
        if requested_version == latest_version:
            raise ReleaseError("Release version is already published as latest")
        make_latest = "true" if requested_version > latest_version else "false"
    checked_release(github.api("GET", f"{base}/releases/{release_id}"), draft=True)
    check_tag()
    result = github.api("PATCH", f"{base}/releases/{release_id}",
                        {"draft": False, "make_latest": make_latest})
    if (result.get("draft") is not False or result.get("tag_name") != tag or
            result.get("target_commitish") != source or result.get("id") != release_id):
        raise ReleaseError("GitHub did not confirm release publication")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--source-commit", required=True)
    parser.add_argument("--directory", required=True, type=Path)
    parser.add_argument("--publish", action="store_true", required=True)
    args = parser.parse_args()
    token = os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN")
    try:
        if not token:
            raise ReleaseError("GH_TOKEN or GITHUB_TOKEN is required")
        result = publish_release(args.repository, args.tag, args.source_commit, args.directory, GitHub(token))
        print(json.dumps({"published": True, "tag": args.tag, "url": result.get("html_url")}))
    except (ReleaseError, OSError, ValueError, KeyError, TypeError, http.client.HTTPException) as exc:
        print(f"Release publication failed: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
