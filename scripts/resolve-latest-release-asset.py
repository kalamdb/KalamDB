#!/usr/bin/env python3

import argparse
import json
import re
import sys
import urllib.error
import urllib.request


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Resolve the first GitHub release asset matching a pattern.",
    )
    parser.add_argument("--repository", required=True, help="GitHub repository in owner/name form")
    parser.add_argument("--github-output", required=True, help="Path to the GitHub Actions output file")
    parser.add_argument("--asset-pattern", required=True, help="Regex for the desired asset name")
    parser.add_argument("--token", default="", help="GitHub token used for the releases API request")
    parser.add_argument("--per-page", type=int, default=20, help="How many releases to fetch")
    return parser.parse_args()


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


def write_outputs(output_path: str, release_tag: str, asset_name: str, asset_api_url: str) -> None:
    with open(output_path, "a", encoding="utf-8") as handle:
        handle.write(f"release_tag={release_tag}\n")
        handle.write(f"asset_name={asset_name}\n")
        handle.write(f"asset_api_url={asset_api_url}\n")


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

    for release in releases:
        if release.get("draft"):
            continue

        for asset in release.get("assets", []):
            name = asset.get("name", "")
            if asset_pattern.match(name):
                write_outputs(args.github_output, release["tag_name"], name, asset["url"])
                return 0

    raise SystemExit("No release asset matched the requested pattern.")


if __name__ == "__main__":
    sys.exit(main())