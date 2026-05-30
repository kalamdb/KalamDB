#!/usr/bin/env python3

from __future__ import annotations

import argparse
import difflib
import json
import os
import re
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

import tomllib


ROOT = Path(__file__).resolve().parents[1]
VERSIONS_PATH = ROOT / "versions.json"
PROTOCOL = "v1"
DEV_CHANNEL_CORE = "main"
DEV_CHANNEL_STABILITY = "dev"

DOCS_VERSIONED_SECTIONS: dict[str, dict[str, Any]] = {
    "server": {
        "folder_name": "server",
        "root_href": "/docs/server",
        "current_core_component": "server",
        "archived": (
            {"slug": "0-4-2-rc-3", "label": "0.4.2-rc.3"},
            {"slug": "0-4-1-beta", "label": "0.4.1-beta"},
        ),
    },
    "pg-extension": {
        "folder_name": "pg-kalam",
        "legacy_folder_name": "postgres-extension",
        "root_href": "/docs/pg-kalam",
        "legacy_root_hrefs": ("/docs/postgres-extension",),
        "current_core_component": "pg_extension",
        "archived": (
            {"slug": "0-4-2-rc-3", "label": "0.4.2-rc.3"},
        ),
    },
    "typescript-sdk": {
        "folder_name": "ts-sdk",
        "root_href": "/docs/ts-sdk",
        "legacy_root_hrefs": ("/docs/sdk/typescript",),
        "legacy_sdk_child_name": "typescript",
        "current_packages": (
            {"source_group": "typescript", "package_name": "@kalamdb/client"},
            {"source_group": "typescript", "package_name": "@kalamdb/consumer"},
            {"source_group": "typescript", "package_name": "@kalamdb/orm"},
            {"source_group": "typescript", "package_name": "@kalamdb/react"},
        ),
        "archived": (
            {"slug": "0-4-2-rc-1", "label": "0.4.2-rc.1"},
            {"slug": "0-4-1-beta", "label": "0.4.1-beta"},
            {"slug": "0-4-x", "label": "0.4.x"},
        ),
    },
    "dart-sdk": {
        "folder_name": "dart-sdk",
        "root_href": "/docs/dart-sdk",
        "legacy_root_hrefs": ("/docs/sdk/dart",),
        "legacy_sdk_child_name": "dart",
        "current_packages": (
            {"source_group": "dart", "package_name": "kalam_link"},
        ),
        "archived": (
            {"slug": "0-4-1-beta-2", "label": "0.4.1-beta.2"},
        ),
    },
}

DOCS_COMPATIBILITY_MATRIX: tuple[dict[str, Any], ...] = (
    {
        "sections": {
            "server": "0-4-2-rc-3",
            "pg-extension": "0-4-2-rc-3",
            "typescript-sdk": "0-4-2-rc-1",
            "dart-sdk": "0-4-1-beta-2",
        },
        "rust_sdk": "Beta source package",
        "notes": "Recommended release-candidate pairing for 0.4.2 testing.",
    },
    {
        "sections": {
            "server": "0-4-1-beta",
            "pg-extension": None,
            "typescript-sdk": "0-4-1-beta",
            "dart-sdk": "0-4-1-beta-2",
        },
        "rust_sdk": "Beta source package",
        "notes": "Use for older beta applications that are not ready to move to 0.4.2 release candidates.",
    },
)

WORKSPACE_CARGO = ROOT / "Cargo.toml"
BACKEND_CARGO = ROOT / "backend" / "Cargo.toml"
CLI_CARGO = ROOT / "cli" / "Cargo.toml"
CLI_NPM_PACKAGE = ROOT / "link" / "sdks" / "typescript" / "cli" / "package.json"
PG_CARGO = ROOT / "pg" / "Cargo.toml"
RUST_SDK_CARGO = ROOT / "link" / "kalam-client" / "Cargo.toml"
PYTHON_PYPROJECT = ROOT / "link" / "sdks" / "python" / "pyproject.toml"
PYTHON_CARGO = ROOT / "link" / "sdks" / "python" / "Cargo.toml"
DART_PUBSPEC = ROOT / "link" / "sdks" / "dart" / "pubspec.yaml"
TS_CLIENT_PACKAGE = ROOT / "link" / "sdks" / "typescript" / "client" / "package.json"
TS_CONSUMER_PACKAGE = ROOT / "link" / "sdks" / "typescript" / "consumer" / "package.json"
TS_ORM_PACKAGE = ROOT / "link" / "sdks" / "typescript" / "orm" / "package.json"
TS_REACT_PACKAGE = ROOT / "link" / "sdks" / "typescript" / "react" / "package.json"

WORKSPACE_VERSION_PATTERNS = (
    re.compile(r"(?m)^\s*version\.workspace\s*=\s*true\s*$"),
    re.compile(r"(?m)^\s*version\s*=\s*\{\s*workspace\s*=\s*true\s*\}\s*$"),
)


class VersionError(RuntimeError):
    pass


def load_toml(path: Path) -> dict[str, Any]:
    return tomllib.loads(path.read_text(encoding="utf-8"))


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def write_json(path: Path, payload: dict[str, Any]) -> None:
    path.write_text(f"{json.dumps(payload, indent=2)}\n", encoding="utf-8")


def read_pubspec_name_and_version(path: Path) -> tuple[str, str]:
    name = None
    version = None
    for line in path.read_text(encoding="utf-8").splitlines():
        if name is None:
            match = re.match(r"^name:\s*(.+?)\s*$", line)
            if match:
                name = match.group(1)
                continue
        if version is None:
            match = re.match(r"^version:\s*(.+?)\s*$", line)
            if match:
                version = match.group(1)
                continue

    if not name or not version:
        raise VersionError(f"Failed to read name/version from {path.relative_to(ROOT)}")

    return name, version


def is_workspace_version_manifest(path: Path) -> bool:
    text = path.read_text(encoding="utf-8")
    return any(pattern.search(text) for pattern in WORKSPACE_VERSION_PATTERNS)


def require_workspace_version(path: Path) -> None:
    if not is_workspace_version_manifest(path):
        raise VersionError(
            f"{path.relative_to(ROOT)} must inherit its version from the workspace root"
        )


def version_stability(version: str) -> str:
    if version == DEV_CHANNEL_CORE:
        return DEV_CHANNEL_STABILITY
    if "-" not in version:
        return "stable"

    prerelease = version.split("-", 1)[1]
    label = prerelease.split(".", 1)[0].lower()
    label = re.sub(r"\d+$", "", label)
    if label.startswith("alpha"):
        return "alpha"
    if label.startswith("beta"):
        return "beta"
    if label.startswith("rc"):
        return "rc"
    return label or "pre-release"


def compatible_core_track(core_version: str) -> str:
    core_base = core_version.split("-", 1)[0]
    parts = core_base.split(".")
    if len(parts) < 2:
        raise VersionError(f"Unsupported core version format: {core_version}")
    return f"{parts[0]}.{parts[1]}.x"


def get_workspace_version() -> str:
    cargo = load_toml(WORKSPACE_CARGO)
    try:
        return cargo["workspace"]["package"]["version"]
    except KeyError as error:
        raise VersionError("Missing [workspace.package].version in Cargo.toml") from error


def get_cargo_package(path: Path) -> dict[str, Any]:
    cargo = load_toml(path)
    try:
        return cargo["package"]
    except KeyError as error:
        raise VersionError(f"Missing [package] section in {path.relative_to(ROOT)}") from error


def get_package_json(path: Path) -> dict[str, Any]:
    return load_json(path)


def get_python_package() -> dict[str, str]:
    pyproject = load_toml(PYTHON_PYPROJECT)
    try:
        project = pyproject["project"]
        return {
            "name": project["name"],
            "version": project["version"],
        }
    except KeyError as error:
        raise VersionError(
            f"Missing [project].name/version in {PYTHON_PYPROJECT.relative_to(ROOT)}"
        ) from error


def filter_internal_dependencies(package_data: dict[str, Any]) -> dict[str, str]:
    dependencies: dict[str, str] = {}
    for section in ("dependencies", "peerDependencies"):
        for name, version in package_data.get(section, {}).items():
            if name.startswith("@kalamdb/"):
                dependencies[name] = version
    return dependencies


def resolve_shared_typescript_version(packages: dict[str, dict[str, Any]]) -> str:
    versions = {name: package["version"] for name, package in packages.items()}
    unique_versions = set(versions.values())
    if len(unique_versions) != 1:
        formatted = ", ".join(
            f"{name}={version}" for name, version in sorted(versions.items())
        )
        raise VersionError(
            "All TypeScript SDK packages must use the same version. "
            f"Found: {formatted}"
        )
    return next(iter(unique_versions))


def build_package_entry(version: str, compatible_core: str, depends_on: dict[str, str] | None = None) -> dict[str, Any]:
    entry: dict[str, Any] = {
        "version": version,
        "compatible_core": compatible_core,
        "protocol": PROTOCOL,
    }
    if depends_on:
        entry["depends_on"] = depends_on
    return entry


def build_docs_manifest() -> dict[str, Any]:
    sections: dict[str, Any] = {}

    for section_id, section in DOCS_VERSIONED_SECTIONS.items():
        rendered_section: dict[str, Any] = {
            "folder_name": section["folder_name"],
            "root_href": section["root_href"],
            "archived": [dict(entry) for entry in section["archived"]],
        }

        if "legacy_folder_name" in section:
            rendered_section["legacy_folder_name"] = section["legacy_folder_name"]

        if "legacy_root_hrefs" in section:
            rendered_section["legacy_root_hrefs"] = list(section["legacy_root_hrefs"])

        if "legacy_sdk_child_name" in section:
            rendered_section["legacy_sdk_child_name"] = section["legacy_sdk_child_name"]

        if "current_core_component" in section:
            rendered_section["current_core_component"] = section["current_core_component"]

        if "current_packages" in section:
            rendered_section["current_packages"] = [
                dict(entry) for entry in section["current_packages"]
            ]

        sections[section_id] = rendered_section

    return {
        "versioning": {
            "sections": sections,
            "compatibility_matrix": [
                {
                    "sections": dict(row["sections"]),
                    "rust_sdk": row["rust_sdk"],
                    "notes": row["notes"],
                }
                for row in DOCS_COMPATIBILITY_MATRIX
            ],
        }
    }


def append_archived_release(existing: dict[str, Any] | None, current_core_version: str) -> list[dict[str, Any]]:
    archived = list(existing.get("archived", [])) if existing else []
    previous_latest = (existing or {}).get("channels", {}).get("latest", {})
    previous_core = previous_latest.get("core")
    if not previous_core or previous_core == current_core_version:
        return archived

    if any(entry.get("core") == previous_core for entry in archived):
        return archived

    archived.append(
        {
            "core": previous_core,
            "protocol": previous_latest.get("protocol", PROTOCOL),
            "released_at": datetime.now(timezone.utc).date().isoformat(),
            "stability": previous_latest.get("stability", version_stability(previous_core)),
        }
    )
    return archived


def build_versions_manifest(existing: dict[str, Any] | None) -> dict[str, Any]:
    require_workspace_version(BACKEND_CARGO)
    require_workspace_version(CLI_CARGO)
    require_workspace_version(PG_CARGO)
    require_workspace_version(RUST_SDK_CARGO)

    core_version = get_workspace_version()
    core_stability = version_stability(core_version)
    compatible_core = compatible_core_track(core_version)

    rust_sdk_package = get_cargo_package(RUST_SDK_CARGO)
    python_package = get_python_package()
    python_native_package = get_cargo_package(PYTHON_CARGO)
    if python_native_package.get("version") != python_package["version"]:
        raise VersionError(
            "link/sdks/python/Cargo.toml and link/sdks/python/pyproject.toml must use the same version"
        )

    dart_name, dart_version = read_pubspec_name_and_version(DART_PUBSPEC)
    cli_npm_package = get_package_json(CLI_NPM_PACKAGE)
    if cli_npm_package.get("version") != core_version:
        raise VersionError(
            "link/sdks/typescript/cli/package.json must use the same version as the workspace root"
        )

    ts_client = get_package_json(TS_CLIENT_PACKAGE)
    ts_consumer = get_package_json(TS_CONSUMER_PACKAGE)
    ts_orm = get_package_json(TS_ORM_PACKAGE)
    ts_react = get_package_json(TS_REACT_PACKAGE)
    typescript_packages = {
        ts_client["name"]: ts_client,
        ts_consumer["name"]: ts_consumer,
        ts_orm["name"]: ts_orm,
        ts_react["name"]: ts_react,
    }
    shared_typescript_version = resolve_shared_typescript_version(typescript_packages)

    return {
        "schema_version": 1,
        "channels": {
            "latest": {
                "core": core_version,
                "protocol": PROTOCOL,
                "stability": core_stability,
            },
            "dev": {
                "core": DEV_CHANNEL_CORE,
                "protocol": PROTOCOL,
                "stability": DEV_CHANNEL_STABILITY,
            },
        },
        "docs": build_docs_manifest(),
        "archived": append_archived_release(existing, core_version),
        "packages": {
            "core_components": {
                "server": {
                    "version": core_version,
                    "compatible_core": core_version,
                    "protocol": PROTOCOL,
                },
                "cli": {
                    "version": core_version,
                    "compatible_core": core_version,
                    "protocol": PROTOCOL,
                },
                "pg_extension": {
                    "version": core_version,
                    "compatible_core": core_version,
                    "protocol": PROTOCOL,
                },
            },
            "typescript": {
                ts_client["name"]: build_package_entry(shared_typescript_version, compatible_core),
                ts_orm["name"]: build_package_entry(
                    shared_typescript_version,
                    compatible_core,
                    filter_internal_dependencies(ts_orm),
                ),
                ts_consumer["name"]: build_package_entry(
                    shared_typescript_version,
                    compatible_core,
                    filter_internal_dependencies(ts_consumer),
                ),
                ts_react["name"]: build_package_entry(
                    shared_typescript_version,
                    compatible_core,
                    filter_internal_dependencies(ts_react),
                ),
            },
            "python": {
                python_package["name"]: build_package_entry(
                    python_package["version"],
                    compatible_core,
                )
            },
            "npm": {
                cli_npm_package["name"]: build_package_entry(
                    cli_npm_package["version"],
                    core_version,
                )
            },
            "rust": {
                rust_sdk_package["name"]: build_package_entry(
                    core_version,
                    core_version,
                )
            },
            "dart": {
                dart_name: build_package_entry(dart_version, compatible_core)
            },
        },
    }


def load_existing_versions() -> dict[str, Any] | None:
    if not VERSIONS_PATH.exists():
        return None
    return load_json(VERSIONS_PATH)


def manifest_as_text(manifest: dict[str, Any]) -> str:
    return f"{json.dumps(manifest, indent=2)}\n"


def ensure_manifest_matches() -> int:
    existing = load_existing_versions()
    if existing is None:
        print("versions.json is missing. Run: python3 scripts/versions.py sync --write", file=sys.stderr)
        return 1

    expected = build_versions_manifest(existing)
    if existing == expected:
        print("versions.json is in sync")
        return 0

    diff = "".join(
        difflib.unified_diff(
            manifest_as_text(existing).splitlines(keepends=True),
            manifest_as_text(expected).splitlines(keepends=True),
            fromfile="versions.json",
            tofile="expected",
        )
    )
    print(diff, file=sys.stderr)
    print("Run: python3 scripts/versions.py sync --write", file=sys.stderr)
    return 1


def sync_manifest(write: bool) -> int:
    expected = build_versions_manifest(load_existing_versions())
    if write:
        write_json(VERSIONS_PATH, expected)
        print(f"Wrote {VERSIONS_PATH.relative_to(ROOT)}")
    else:
        sys.stdout.write(manifest_as_text(expected))
    return 0


def github_outputs(repository: str | None) -> dict[str, str]:
    manifest = build_versions_manifest(load_existing_versions())
    core_version = manifest["channels"]["latest"]["core"]
    release_tag = f"v{core_version}"
    outputs = {
        "core_version": core_version,
        "root_version": core_version,
        "version": core_version,
        "release_tag": release_tag,
        "tag": release_tag,
        "pre_release": str(version_stability(core_version) != "stable").lower(),
        "core_stability": manifest["channels"]["latest"]["stability"],
        "typescript_version": manifest["packages"]["typescript"]["@kalamdb/client"]["version"],
        "dart_version": next(iter(manifest["packages"]["dart"].values()))["version"],
        "python_version": next(iter(manifest["packages"]["python"].values()))["version"],
        "npm_cli_version": manifest["packages"]["npm"]["@kalamdb/cli"]["version"],
        "rust_sdk_version": next(iter(manifest["packages"]["rust"].values()))["version"],
        "release_server_asset_name": f"kalamdb-server-{core_version}-linux-x86_64.tar.gz",
    }
    if repository:
        outputs["release_server_asset_url"] = (
            f"https://github.com/{repository}/releases/download/{release_tag}/{outputs['release_server_asset_name']}"
        )
    return outputs


def emit_github_outputs(output_path: Path | None, repository: str | None) -> int:
    outputs = github_outputs(repository)
    lines = [f"{key}={value}" for key, value in outputs.items()]
    rendered = "\n".join(lines) + "\n"

    if output_path is None:
        sys.stdout.write(rendered)
        return 0

    with output_path.open("a", encoding="utf-8") as handle:
        handle.write(rendered)
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Manage the root versions.json manifest")
    subparsers = parser.add_subparsers(dest="command", required=True)

    verify_parser = subparsers.add_parser("verify", help="Fail if versions.json is out of sync")
    verify_parser.set_defaults(func=lambda args: ensure_manifest_matches())

    sync_parser = subparsers.add_parser("sync", help="Regenerate versions.json")
    sync_parser.add_argument("--write", action="store_true", help="Write the generated manifest to versions.json")
    sync_parser.set_defaults(func=lambda args: sync_manifest(args.write))

    outputs_parser = subparsers.add_parser("github-outputs", help="Emit GitHub Actions outputs")
    outputs_parser.add_argument(
        "--github-output",
        type=Path,
        default=Path(os.environ["GITHUB_OUTPUT"]) if "GITHUB_OUTPUT" in os.environ else None,
        help="Path to the GitHub Actions output file",
    )
    outputs_parser.add_argument(
        "--repository",
        default=os.environ.get("GITHUB_REPOSITORY"),
        help="GitHub owner/repo used to derive release URLs",
    )
    outputs_parser.set_defaults(
        func=lambda args: emit_github_outputs(args.github_output, args.repository)
    )

    return parser


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()
    try:
        return args.func(args)
    except VersionError as error:
        print(f"version sync error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())