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