#!/usr/bin/env bash
# publish.sh - Publish the kalam-client Rust SDK and its crates.io dependencies
#
# Usage:
#   ./publish.sh [OPTIONS]
#
# Options:
#   --dry-run       Run cargo publish --dry-run only
#   --skip-check    Skip cargo check before packaging
#   --version VER   Override workspace crate versions for publish
#
# Environment:
#   CARGO_REGISTRY_TOKEN   crates.io token (required for real publish)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
CRATE_MANIFEST="$REPO_ROOT/link/kalam-client/Cargo.toml"

# Optional .env override; cargo login (~/.cargo/credentials.toml) also works.
if [[ -z "${CARGO_REGISTRY_TOKEN:-}" ]]; then
  ENV_FILE="$SCRIPT_DIR/.env"
  if [[ -f "$ENV_FILE" ]]; then
    CARGO_REGISTRY_TOKEN="$(
      grep -E '^CARGO_REGISTRY_TOKEN=' "$ENV_FILE" | head -n1 | cut -d'=' -f2- | tr -d '[:space:]'
    )"
    if [[ -n "$CARGO_REGISTRY_TOKEN" ]]; then
      echo "Using CARGO_REGISTRY_TOKEN from .env"
    fi
  fi
fi
if [[ -n "${CARGO_REGISTRY_TOKEN:-}" ]]; then
  export CARGO_REGISTRY_TOKEN
fi

cargo_registry_authenticated() {
  if [[ -n "${CARGO_REGISTRY_TOKEN:-}" ]]; then
    return 0
  fi

  local cargo_home="${CARGO_HOME:-$HOME/.cargo}"
  local creds="$cargo_home/credentials.toml"
  [[ -f "$creds" ]] && grep -qE 'token[[:space:]]*=' "$creds"
}

DRY_RUN=false
SKIP_CHECK=false
VERSION_OVERRIDE=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run)
      DRY_RUN=true
      shift
      ;;
    --skip-check)
      SKIP_CHECK=true
      shift
      ;;
    --version)
      VERSION_OVERRIDE="$2"
      shift 2
      ;;
    *)
      echo "Unknown option: $1"
      echo "Usage: $0 [--dry-run] [--skip-check] [--version VERSION]"
      exit 1
      ;;
  esac
done

resolve_version() {
  python3 - <<'PY' "$CRATE_MANIFEST" "$REPO_ROOT/Cargo.toml"
import pathlib
import sys
import tomllib

crate = tomllib.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
package = crate.get("package", {})
version = package.get("version")
if isinstance(version, dict) and version.get("workspace") is True:
    version = None
if version is None:
    workspace = tomllib.loads(pathlib.Path(sys.argv[2]).read_text(encoding="utf-8"))
    version = workspace["workspace"]["package"]["version"]
if not isinstance(version, str) or not version:
    raise SystemExit("failed to resolve crate version")
print(version)
PY
}

if [[ -n "$VERSION_OVERRIDE" ]]; then
  echo "Publishing with version override: $VERSION_OVERRIDE"
  VERSION="$VERSION_OVERRIDE"
else
  VERSION="$(resolve_version)"
fi

echo "════════════════════════════════════════"
echo "  kalam-client crates.io publish"
echo "  Version : $VERSION"
echo "  Dry-run : $DRY_RUN"
echo "════════════════════════════════════════"

SYNC_VERSIONS_SCRIPT="$REPO_ROOT/link/sdks/sync-versions.sh"

prepare_publish_crates() {
  if [[ -n "$VERSION_OVERRIDE" ]]; then
    bash "$SYNC_VERSIONS_SCRIPT" --rust-publish-deps --version "$VERSION"
  else
    bash "$SYNC_VERSIONS_SCRIPT" --rust-publish-deps
  fi
  cp "$REPO_ROOT/link/sdks/rust/README.md" "$REPO_ROOT/link/kalam-client/README.md"
}

restore_publish_crates() {
  bash "$SYNC_VERSIONS_SCRIPT" --rust-publish-deps-restore
}

prepare_publish_crates
trap restore_publish_crates EXIT

if [[ "$SKIP_CHECK" == "false" ]]; then
  echo "Checking kalam-client..."
  (cd "$REPO_ROOT" && cargo check -p kalam-client --features native-sdk,consumer)
fi

if [[ "$DRY_RUN" != "true" ]] && ! cargo_registry_authenticated; then
  echo "No crates.io credentials found."
  echo "Run 'cargo login' or set CARGO_REGISTRY_TOKEN (optionally in $SCRIPT_DIR/.env)."
  exit 1
fi

verify_crate_publish_metadata() {
  local package="$1"
  python3 - <<'PY' "$package" "$REPO_ROOT"
import pathlib
import sys
import tomllib

package, repo_root = sys.argv[1], pathlib.Path(sys.argv[2])
manifests = {
    "kalamdb-observability": repo_root / "backend/crates/kalamdb-observability/Cargo.toml",
    "kalamdb-commons": repo_root / "backend/crates/kalamdb-commons/Cargo.toml",
    "link-common": repo_root / "link/link-common/Cargo.toml",
    "kalam-client": repo_root / "link/kalam-client/Cargo.toml",
}
manifest = tomllib.loads(manifests[package].read_text(encoding="utf-8"))
meta = manifest.get("package", {})
if not meta.get("description"):
    raise SystemExit(f"{package}: missing package description")
readme = meta.get("readme")
if readme:
    readme_path = manifests[package].parent / readme
    if not readme_path.exists():
        raise SystemExit(f"{package}: readme not found at {readme_path}")
print(f"Manifest metadata OK for {package}")
PY
}

publish_package() {
  local package="$1"
  shift
  local -a cmd=(cargo publish -p "$package" --allow-dirty)
  local output
  local status

  if [[ "$DRY_RUN" == "true" ]]; then
    cmd+=(--dry-run)
  fi

  if [[ $# -gt 0 ]]; then
    cmd+=("$@")
  fi

  echo "Running: ${cmd[*]}"
  set +e
  output="$(
    cd "$REPO_ROOT"
    "${cmd[@]}" 2>&1
  )"
  status=$?
  set -e
  echo "$output"

  if [[ $status -ne 0 ]]; then
    if [[ "$DRY_RUN" == "true" ]] && echo "$output" | grep -q 'no matching package named'; then
      echo "Dry-run note: $package depends on crates not yet on crates.io."
      echo "Publishing in order without --dry-run will upload upstream crates first."
      verify_crate_publish_metadata "$package"
      return 0
    fi
    return "$status"
  fi

  if [[ "$DRY_RUN" != "true" ]]; then
    echo "Waiting for crates.io index to update after ${package}..."
    sleep 45
  fi
}

# Publish internal dependencies first so path deps resolve to crates.io versions.
publish_package kalamdb-observability
publish_package kalamdb-commons
publish_package link-common
publish_package kalam-client --features native-sdk

if [[ "$DRY_RUN" == "true" ]]; then
  echo "Dry-run complete. Packaging looks valid."
else
  echo "Published kalam-client@$VERSION to crates.io"
  echo "https://crates.io/crates/kalam-client"
fi
