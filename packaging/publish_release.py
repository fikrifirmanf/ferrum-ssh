#!/usr/bin/env python3
"""Publish verified CircleCI artifacts to a GitHub release for an existing tag."""
from __future__ import annotations

import hashlib
import json
import os
import re
import subprocess
import sys
import tomllib
from pathlib import Path
from urllib.parse import quote

REPOSITORY = "fikrifirmanf/ferrum-ssh"
# Release assets are only accepted on the uploads host; api.github.com answers 404.
UPLOADS = "https://uploads.github.com"
MAX_ASSET_BYTES = 256 * 1024 * 1024


class ReleaseError(Exception):
    """An actionable release validation or publishing failure."""


class GitHub:
    def command(self, *arguments: str, payload: dict | None = None) -> str:
        argv = ["gh", *arguments]
        if payload is not None:
            argv.extend(["--input", "-"])
        try:
            result = subprocess.run(
                argv, input=json.dumps(payload) if payload is not None else None,
                capture_output=True, text=True, check=True, timeout=300,
            )
        except subprocess.CalledProcessError as error:
            raise ReleaseError(error.stderr.strip() or "GitHub rejected the release request.") from error
        except (OSError, subprocess.TimeoutExpired) as error:
            raise ReleaseError("GitHub CLI is unavailable or its request timed out.") from error
        return result.stdout

    def api(self, method: str, path: str, payload: dict | None = None):
        output = self.command("api", "--method", method, path, payload=payload)
        if not output.strip():
            return None
        try:
            return json.loads(output)
        except json.JSONDecodeError as error:
            raise ReleaseError("GitHub returned an invalid JSON response.") from error

    def upload_asset(self, release_id: int, path: Path) -> None:
        endpoint = (
            f"{UPLOADS}/repos/{REPOSITORY}/releases/{release_id}/assets"
            f"?name={quote(path.name, safe='')}"
        )
        try:
            subprocess.run(
                ["gh", "api", "--method", "POST", endpoint,
                 "--header", "Content-Type: application/octet-stream", "--input", str(path)],
                capture_output=True, text=True, check=True, timeout=300,
            )
        except subprocess.CalledProcessError as error:
            raise ReleaseError(error.stderr.strip() or f"GitHub rejected {path.name}.") from error
        except (OSError, subprocess.TimeoutExpired) as error:
            raise ReleaseError("GitHub CLI is unavailable or its request timed out.") from error

    def release(self, tag: str) -> dict | None:
        try:
            return self.api("GET", f"repos/{REPOSITORY}/releases/tags/{quote(tag, safe='')}")
        except ReleaseError as error:
            if "HTTP 404" not in str(error):
                raise
        # The by-tag endpoint hides drafts, so find a draft left by an earlier failed run.
        releases = self.api("GET", f"repos/{REPOSITORY}/releases?per_page=100") or []
        return next((r for r in releases if r.get("draft") and r.get("tag_name") == tag), None)


def prepare_assets(directory: Path, cargo: Path, tag: str) -> dict[str, str]:
    with cargo.open("rb") as source:
        version = tomllib.load(source)["package"]["version"]
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?", version):
        raise ReleaseError("Cargo.toml must contain a valid release version.")
    if tag != f"v{version}":
        raise ReleaseError(f"Tag {tag!r} must match Cargo.toml version v{version}.")
    required = {
        f"Ferrum-{version}-macos-arm64.dmg",
        f"Ferrum-{version}-macos-x86_64.dmg",
        f"Ferrum-{version}-linux-x86_64.tar.gz",
        f"Ferrum-{version}-windows-x86_64.zip",
        f"Ferrum-{version}-windows-x86_64-setup.exe",
    }
    optional = {f"Ferrum-{version}-linux-x86_64.AppImage"}
    files = {p.name for p in directory.iterdir() if p.name != "SHA256SUMS.txt"}
    missing = required - files
    unexpected = files - required - optional
    if missing:
        raise ReleaseError("Missing release packages: " + ", ".join(sorted(missing)))
    if unexpected:
        raise ReleaseError("Unexpected release files: " + ", ".join(sorted(unexpected)))
    digests = {}
    for name in sorted(files):
        path = directory / name
        if path.is_symlink() or not path.is_file() or not 0 < path.stat().st_size <= MAX_ASSET_BYTES:
            raise ReleaseError(f"Invalid release package: {name}")
        with path.open("rb") as source:
            digests[name] = hashlib.file_digest(source, "sha256").hexdigest()
    checksums = directory / "SHA256SUMS.txt"
    checksums.write_text("".join(f"{digest}  {name}\n" for name, digest in digests.items()))
    digests[checksums.name] = hashlib.sha256(checksums.read_bytes()).hexdigest()
    return digests


def verify_tag(github: GitHub, tag: str, commit: str) -> None:
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ReleaseError("CIRCLE_SHA1 must identify the commit that was built.")
    ref = github.api("GET", f"repos/{REPOSITORY}/git/ref/tags/{quote(tag, safe='')}")
    obj = ref["object"]
    for _ in range(5):
        if obj.get("type") != "tag":
            break
        obj = github.api("GET", f"repos/{REPOSITORY}/git/tags/{obj['sha']}")["object"]
    if obj.get("type") != "commit" or obj.get("sha") != commit:
        raise ReleaseError("The GitHub tag does not point to the commit CircleCI built.")


def verify_uploads(release: dict, directory: Path, digests: dict[str, str]) -> None:
    assets = release.get("assets", [])
    names = [asset.get("name") for asset in assets]
    if len(names) != len(set(names)) or set(names) != set(digests):
        raise ReleaseError("GitHub release assets do not match the complete installer set.")
    for asset in assets:
        name = asset["name"]
        if asset.get("size") != (directory / name).stat().st_size:
            raise ReleaseError(f"GitHub reports a different size for {name}.")
        if asset.get("digest") != f"sha256:{digests[name]}":
            raise ReleaseError(f"GitHub checksum verification failed for {name}.")


def publish(github: GitHub, directory: Path, cargo: Path, tag: str, commit: str) -> str:
    digests = prepare_assets(directory, cargo, tag)
    verify_tag(github, tag, commit)
    release = github.release(tag)
    if release is not None and not release.get("draft", False):
        verify_uploads(release, directory, digests)
        return release["html_url"]  # A successful rerun never modifies a published release.
    prerelease = "-" in tag.split("+", 1)[0]
    if release is None:
        release = github.api("POST", f"repos/{REPOSITORY}/releases", {
            "tag_name": tag, "target_commitish": commit, "name": f"Ferrum {tag}",
            "draft": True, "prerelease": prerelease, "generate_release_notes": True,
        })
    if not isinstance(release.get("id"), int) or release.get("tag_name") != tag or not release.get("draft"):
        raise ReleaseError("GitHub did not return the expected draft release.")
    unknown = {a.get("name") for a in release.get("assets", [])} - set(digests)
    if unknown:
        raise ReleaseError("The existing draft contains other files. Review it before retrying.")
    # Partial uploads from an earlier run would make the new uploads fail with "already_exists".
    for asset in release.get("assets", []):
        github.api("DELETE", f"repos/{REPOSITORY}/releases/assets/{asset['id']}")
    # unslop-ignore NO_N_PLUS_ONE -- GitHub's asset upload endpoint accepts exactly one binary per request.
    for name in digests:
        github.upload_asset(release["id"], directory / name)
    release = github.api("GET", f"repos/{REPOSITORY}/releases/{release['id']}")
    verify_uploads(release, directory, digests)
    published = github.api("PATCH", f"repos/{REPOSITORY}/releases/{release['id']}", {
        "draft": False, "prerelease": prerelease, "make_latest": "false" if prerelease else "true",
    })
    if published.get("draft") or published.get("tag_name") != tag:
        raise ReleaseError("GitHub did not confirm publication.")
    return published["html_url"]


def main() -> int:
    if not os.environ.get("CIRCLE_TAG"):
        print("Branch build: GitHub release publishing skipped.")
        return 0
    if not os.environ.get("GH_TOKEN", "").strip():
        print("Set GH_TOKEN in CircleCI with Contents: read and write access to this repository.", file=sys.stderr)
        return 1
    try:
        directory = Path(sys.argv[1] if len(sys.argv) > 1 else "release-artifacts").resolve()
        url = publish(GitHub(), directory, Path("Cargo.toml"), os.environ["CIRCLE_TAG"], os.environ.get("CIRCLE_SHA1", ""))
    except (ReleaseError, OSError, ValueError, KeyError, TypeError) as error:
        print(f"Release not published: {error}", file=sys.stderr)
        return 1
    print(f"Published release: {url}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
