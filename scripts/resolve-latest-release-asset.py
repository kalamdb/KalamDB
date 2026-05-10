#!/usr/bin/env python3

import argparse
import json
import re
import sys
import urllib.error
import urllib.request
from dataclasses import dataclass
from functools import cmp_to_key


SEMVER_RE = re.compile(
    r"^v?(?P<major>0|[1-9]\d*)\.(?P<minor>0|[1-9]\d*)\.(?P<patch>0|[1-9]\d*)"
    r"(?:-(?P<prerelease>[0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$"
)


@dataclass(frozen=True)
class SemVer:
    major: int
    minor: int
    patch: int
    prerelease: tuple[str, ...]
    raw: str


@dataclass(frozen=True)
class ReleaseCandidate:
    tag_name: str
    asset_name: str
    asset_api_url: str
    version: SemVer | None


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Resolve the best GitHub release asset matching a pattern.",
    )
    parser.add_argument("--repository", required=True, help="GitHub repository in owner/name form")
    parser.add_argument("--github-output", required=True, help="Path to the GitHub Actions output file")
    parser.add_argument("--asset-pattern", required=True, help="Regex for the desired asset name")
    parser.add_argument(
        "--preferred-version",
        default="",
        help="Prefer this exact release version first, then the nearest compatible track",
    )
    parser.add_argument("--token", default="", help="GitHub token used for the releases API request")
    parser.add_argument("--per-page", type=int, default=100, help="How many releases to fetch")
    return parser.parse_args()


def parse_semver(version_text: str) -> SemVer | None:
    match = SEMVER_RE.match(version_text.strip())
    if not match:
        return None

    prerelease = tuple(filter(None, (match.group("prerelease") or "").split(".")))
    return SemVer(
        major=int(match.group("major")),
        minor=int(match.group("minor")),
        patch=int(match.group("patch")),
        prerelease=prerelease,
        raw=version_text.lstrip("v"),
    )


def compare_prerelease_identifier(left: str, right: str) -> int:
    left_is_numeric = left.isdigit()
    right_is_numeric = right.isdigit()
    if left_is_numeric and right_is_numeric:
        left_value = int(left)
        right_value = int(right)
        return (left_value > right_value) - (left_value < right_value)
    if left_is_numeric != right_is_numeric:
        return -1 if left_is_numeric else 1
    return (left > right) - (left < right)


def compare_semver(left: SemVer, right: SemVer) -> int:
    core_left = (left.major, left.minor, left.patch)
    core_right = (right.major, right.minor, right.patch)
    if core_left != core_right:
        return (core_left > core_right) - (core_left < core_right)

    left_has_prerelease = bool(left.prerelease)
    right_has_prerelease = bool(right.prerelease)
    if left_has_prerelease != right_has_prerelease:
        return -1 if left_has_prerelease else 1
    if not left_has_prerelease:
        return 0

    for left_part, right_part in zip(left.prerelease, right.prerelease):
        result = compare_prerelease_identifier(left_part, right_part)
        if result != 0:
            return result

    return (len(left.prerelease) > len(right.prerelease)) - (
        len(left.prerelease) < len(right.prerelease)
    )


def compare_candidate_version(left: ReleaseCandidate, right: ReleaseCandidate) -> int:
    if left.version and right.version:
        return compare_semver(left.version, right.version)
    if left.version:
        return 1
    if right.version:
        return -1
    return 0


def newest_candidate(candidates: list[ReleaseCandidate]) -> ReleaseCandidate:
    return max(candidates, key=cmp_to_key(compare_candidate_version))


def track_matches(candidate: SemVer, preferred: SemVer) -> bool:
    return candidate.major == preferred.major and candidate.minor == preferred.minor


def choose_candidate(
    candidates: list[ReleaseCandidate], preferred_version_text: str,
) -> tuple[ReleaseCandidate, str]:
    preferred = parse_semver(preferred_version_text) if preferred_version_text else None
    parseable = [candidate for candidate in candidates if candidate.version is not None]

    if preferred is not None:
        exact_matches = [candidate for candidate in parseable if candidate.version == preferred]
        if exact_matches:
            return newest_candidate(exact_matches), "exact_version"

        same_track = [
            candidate
            for candidate in parseable
            if track_matches(candidate.version, preferred)
        ]
        if same_track:
            return newest_candidate(same_track), "same_track"

    if parseable:
        return newest_candidate(parseable), "latest_available"

    return candidates[0], "api_order_fallback"


def build_candidates(releases: list[dict], asset_pattern: re.Pattern[str]) -> list[ReleaseCandidate]:
    candidates: list[ReleaseCandidate] = []

    for release in releases:
        if release.get("draft"):
            continue

        tag_name = str(release.get("tag_name", ""))
        version = parse_semver(tag_name)
        for asset in release.get("assets", []):
            name = asset.get("name", "")
            if asset_pattern.match(name):
                candidates.append(
                    ReleaseCandidate(
                        tag_name=tag_name,
                        asset_name=name,
                        asset_api_url=asset["url"],
                        version=version,
                    )
                )

    return candidates


def fetch_releases(repository: str, token: str, per_page: int) -> list[dict]:
    api_url = f"https://api.github.com/repos/{repository}/releases?per_page={per_page}"
    headers = {"Accept": "application/vnd.github+json"}
    if token:
        headers["Authorization"] = f"Bearer {token}"

    request = urllib.request.Request(api_url, headers=headers)
    with urllib.request.urlopen(request) as response:
        payload = json.load(response)

    if not isinstance(payload, list):
        raise RuntimeError(f"Unexpected releases payload type: {type(payload).__name__}")

    return payload


def write_outputs(
    output_path: str,
    release_tag: str,
    asset_name: str,
    asset_api_url: str,
    selected_version: str,
    selection_reason: str,
) -> None:
    with open(output_path, "a", encoding="utf-8") as handle:
        handle.write(f"release_tag={release_tag}\n")
        handle.write(f"asset_name={asset_name}\n")
        handle.write(f"asset_api_url={asset_api_url}\n")
        handle.write(f"selected_version={selected_version}\n")
        handle.write(f"selection_reason={selection_reason}\n")


def main() -> int:
    args = parse_args()
    asset_pattern = re.compile(args.asset_pattern)

    try:
        releases = fetch_releases(args.repository, args.token, args.per_page)
    except urllib.error.HTTPError as error:
        body = error.read().decode("utf-8", errors="replace")
        raise SystemExit(f"GitHub releases API request failed with {error.code}: {body}") from error
    except urllib.error.URLError as error:
        raise SystemExit(f"GitHub releases API request failed: {error.reason}") from error

    candidates = build_candidates(releases, asset_pattern)
    if not candidates:
        raise SystemExit("No release asset matched the requested pattern.")

    selected, selection_reason = choose_candidate(candidates, args.preferred_version)
    selected_version = selected.version.raw if selected.version is not None else selected.tag_name
    write_outputs(
        args.github_output,
        selected.tag_name,
        selected.asset_name,
        selected.asset_api_url,
        selected_version,
        selection_reason,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())