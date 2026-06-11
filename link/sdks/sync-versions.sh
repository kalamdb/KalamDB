#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SDKS_DIR="$SCRIPT_DIR"
TS_DIR="$SDKS_DIR/typescript"
ROOT_DIR="$(cd "$SDKS_DIR/../.." && pwd)"
ROOT_CARGO="$ROOT_DIR/Cargo.toml"
VERSIONS_SCRIPT="$ROOT_DIR/scripts/versions.py"
PYTHON_PYPROJECT="$SDKS_DIR/python/pyproject.toml"
PYTHON_CARGO="$SDKS_DIR/python/Cargo.toml"
DART_PUBSPEC="$SDKS_DIR/dart/pubspec.yaml"

RUST_PUBLISH_MODE=""
RUST_PUBLISH_VERSION_OVERRIDE=""

usage() {
  cat <<'EOF'
Usage:
  bash link/sdks/sync-versions.sh
  bash link/sdks/sync-versions.sh --rust-publish-deps [--version VERSION]
  bash link/sdks/sync-versions.sh --rust-publish-deps-restore

Default mode syncs TypeScript, Dart, Python, and versions.json to the root
Cargo workspace version.

Rust publish modes update [workspace.dependencies] path entries used by the
kalam-client crates.io publish chain:
  kalamdb-observability -> kalamdb-commons -> link-common -> kalam-client
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --rust-publish-deps)
      RUST_PUBLISH_MODE="apply"
      shift
      ;;
    --rust-publish-deps-restore)
      RUST_PUBLISH_MODE="restore"
      shift
      ;;
    --version)
      RUST_PUBLISH_VERSION_OVERRIDE="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

resolve_workspace_version() {
  python3 - "$ROOT_CARGO" <<'PY'
import sys
import tomllib
from pathlib import Path

workspace = tomllib.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
version = workspace["workspace"]["package"]["version"]
if not isinstance(version, str) or not version:
    raise SystemExit("failed to resolve workspace version")
print(version)
PY
}

sync_rust_publish_dependency_versions() {
  local mode="$1"
  local version="${2:-}"

  python3 - "$ROOT_CARGO" "$mode" "$version" <<'PY'
import re
import sys
from pathlib import Path

cargo_path = Path(sys.argv[1])
mode = sys.argv[2]
version = sys.argv[3]

dependency_names = (
    "kalamdb-observability",
    "kalamdb-commons",
    "link-common",
)

lines = cargo_path.read_text(encoding="utf-8").splitlines()
in_workspace_dependencies = False
changed = False
found = 0

for index, line in enumerate(lines):
    if re.match(r"^\[workspace\.dependencies\]\s*$", line):
        in_workspace_dependencies = True
        continue

    if in_workspace_dependencies and re.match(r"^\[[^\]]+\]\s*$", line):
        in_workspace_dependencies = False

    if not in_workspace_dependencies:
        continue

    for dependency_name in dependency_names:
        prefix = f"{dependency_name} = {{"
        if not line.strip().startswith(prefix):
            continue

        if mode == "apply":
            if not version:
                raise SystemExit("missing version for --rust-publish-deps")
            if re.search(r'\bversion\s*=\s*"[^"]+"', line):
                updated = re.sub(
                    r'\bversion\s*=\s*"[^"]+"',
                    f'version = "{version}"',
                    line,
                    count=1,
                )
            else:
                updated = re.sub(
                    r'(\bpath\s*=\s*"[^"]+")\s*,',
                    rf'\1, version = "{version}",',
                    line,
                    count=1,
                )
        elif mode == "restore":
            updated = re.sub(
                r',\s*version\s*=\s*"[^"]+"\s*',
                "",
                line,
                count=1,
            )
            updated = re.sub(r",\s{2,}", ", ", updated)
        else:
            raise SystemExit(f"unsupported rust publish mode: {mode}")

        found += 1
        if updated != line:
            lines[index] = updated
            changed = True
        break

if found != len(dependency_names):
    raise SystemExit(
        "failed to locate all Rust SDK workspace path dependencies in [workspace.dependencies]"
    )

cargo_path.write_text("\n".join(lines) + "\n", encoding="utf-8")
if changed:
    print("Updated root Cargo.toml workspace path dependency versions")
elif mode == "restore":
    print("Rust publish path dependency versions already absent")
else:
    print("Rust publish path dependency versions already up to date")
PY
}

if [[ -n "$RUST_PUBLISH_MODE" ]]; then
  if [[ "$RUST_PUBLISH_MODE" == "apply" ]]; then
    if [[ -n "$RUST_PUBLISH_VERSION_OVERRIDE" ]]; then
      VERSION="$RUST_PUBLISH_VERSION_OVERRIDE"
    else
      VERSION="$(resolve_workspace_version)"
    fi
    echo "Applying crates.io path dependency versions for Rust SDK publish: $VERSION"
    sync_rust_publish_dependency_versions apply "$VERSION"
  else
    echo "Restoring workspace path dependencies for local development"
    sync_rust_publish_dependency_versions restore ""
  fi
  exit 0
fi

TS_PACKAGE_JSON_FILES=(
    "$TS_DIR/cli/package.json"
  "$TS_DIR/client/package.json"
  "$TS_DIR/consumer/package.json"
  "$TS_DIR/orm/package.json"
  "$TS_DIR/react/package.json"
)

for file in "$ROOT_CARGO" "$VERSIONS_SCRIPT" "$PYTHON_PYPROJECT" "$PYTHON_CARGO" "$DART_PUBSPEC" "${TS_PACKAGE_JSON_FILES[@]}"; do
  if [[ ! -f "$file" ]]; then
    echo "Missing required file: $file" >&2
    exit 1
  fi
done

VERSION_INFO_RAW="$({
  python3 - "$ROOT_CARGO" <<'PY'
import sys
import tomllib
from pathlib import Path

cargo_path = Path(sys.argv[1])
workspace = tomllib.loads(cargo_path.read_text(encoding="utf-8"))
version = workspace["workspace"]["package"]["version"]
base = version.split("-", 1)[0]
major, minor, patch = base.split(".")
upper_bound = f"{major}.{int(minor) + 1}.0"
lower_bound = f"{base}-0" if "-" in version else base

print(version)
print(f">={lower_bound} <{upper_bound}")
PY
})"

ROOT_VERSION="${VERSION_INFO_RAW%%$'\n'*}"
INTERNAL_RANGE="${VERSION_INFO_RAW#*$'\n'}"

echo "Syncing SDK package versions to root Cargo workspace version $ROOT_VERSION"
echo "Using TypeScript internal peer dependency range $INTERNAL_RANGE"

python3 - \
  "$ROOT_VERSION" \
  "$INTERNAL_RANGE" \
  "$ROOT_DIR" \
    "${PUBLISH_SCOPE_OVERRIDE:-}" \
  "$DART_PUBSPEC" \
  "$PYTHON_PYPROJECT" \
  "$PYTHON_CARGO" \
  "${TS_PACKAGE_JSON_FILES[@]}" <<'PY'
import json
import re
import sys
from pathlib import Path

root_version = sys.argv[1]
internal_range = sys.argv[2]
root_dir = Path(sys.argv[3])
typescript_scope_override = sys.argv[4].strip()
dart_pubspec = Path(sys.argv[5])
python_pyproject = Path(sys.argv[6])
python_cargo = Path(sys.argv[7])
typescript_packages = [Path(value) for value in sys.argv[8:]]
internal_packages = {
    "@kalamdb/client",
    "@kalamdb/consumer",
    "@kalamdb/orm",
    "@kalamdb/react",
}


def display(path: Path) -> str:
    try:
        return str(path.relative_to(root_dir))
    except ValueError:
        return str(path)


def display_package_name(package_name: str) -> str:
    if not typescript_scope_override or not package_name.startswith("@") or "/" not in package_name:
        return package_name

    _, package_suffix = package_name.split("/", 1)
    return f"{typescript_scope_override}/{package_suffix}"


def update_yaml_version(path: Path, new_version: str) -> None:
    text = path.read_text(encoding="utf-8")
    updated, count = re.subn(r"(?m)^version:\s*.+$", f"version: {new_version}", text, count=1)
    if count != 1:
        raise SystemExit(f"Failed to update version in {display(path)}")
    path.write_text(updated, encoding="utf-8")
    print(f"Updated {display(path)}: version -> {new_version}")


def update_toml_section_version(path: Path, section_name: str, new_version: str) -> None:
    lines = path.read_text(encoding="utf-8").splitlines(keepends=True)
    current_section = None
    updated = False

    for index, line in enumerate(lines):
        section_match = re.match(r"^\s*\[([^\]]+)\]\s*$", line)
        if section_match:
            current_section = section_match.group(1)
            continue

        if current_section == section_name and re.match(r"^\s*version\s*=", line):
            indent = re.match(r"^(\s*)", line).group(1)
            lines[index] = f'{indent}version = "{new_version}"\n'
            updated = True
            break

    if not updated:
        raise SystemExit(f"Failed to update [{section_name}] version in {display(path)}")

    path.write_text("".join(lines), encoding="utf-8")
    print(f"Updated {display(path)}: [{section_name}] version -> {new_version}")


for package_path in typescript_packages:
    package_data = json.loads(package_path.read_text(encoding="utf-8"))
    package_name = package_data["name"]
    rendered_package_name = display_package_name(package_name)
    package_data["version"] = root_version

    peer_dependencies = package_data.get("peerDependencies")
    if isinstance(peer_dependencies, dict):
        for dependency_name in list(peer_dependencies.keys()):
            if dependency_name in internal_packages:
                peer_dependencies[dependency_name] = internal_range

    package_path.write_text(f"{json.dumps(package_data, indent=2)}\n", encoding="utf-8")
    print(f"Updated {rendered_package_name} ({display(package_path)}): version -> {root_version}")

update_yaml_version(dart_pubspec, root_version)
update_toml_section_version(python_pyproject, "project", root_version)
update_toml_section_version(python_cargo, "package", root_version)
PY

python3 "$VERSIONS_SCRIPT" sync --write
python3 "$VERSIONS_SCRIPT" verify

echo "SDK package manifests and versions.json are in sync."