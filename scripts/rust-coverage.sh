#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
BACKEND_MANIFEST="$ROOT_DIR/backend/Cargo.toml"
SCOPE="${KALAMDB_COVERAGE_SCOPE:-backend}"
INCLUDE_E2E="${KALAMDB_COVERAGE_INCLUDE_E2E:-false}"
OUTPUT_DIR="${KALAMDB_COVERAGE_DIR:-$ROOT_DIR/target/coverage/$SCOPE}"
LCOV_PATH="$OUTPUT_DIR/lcov.info"
HTML_DIR="$OUTPUT_DIR/html"

build_backend_package_args() {
  local metadata_json
  metadata_json="$(cargo metadata --manifest-path "$ROOT_DIR/Cargo.toml" --no-deps --format-version 1)"

  BACKEND_PACKAGES=()
  while IFS= read -r pkg; do
    [[ -n "$pkg" ]] && BACKEND_PACKAGES+=("$pkg")
  done < <(
    METADATA_JSON="$metadata_json" ROOT_DIR="$ROOT_DIR" python3 - <<'PY'
import json
import os

root_dir = os.environ["ROOT_DIR"].rstrip("/") + "/"
metadata = json.loads(os.environ["METADATA_JSON"])

for pkg in metadata.get("packages", []):
    manifest_path = pkg.get("manifest_path", "")
    if manifest_path.startswith(root_dir + "backend/"):
        print(pkg["name"])
PY
  )

  if [[ ${#BACKEND_PACKAGES[@]} -eq 0 ]]; then
    echo "Failed to resolve backend package list from cargo metadata." >&2
    exit 1
  fi

  PACKAGE_ARGS=()
  for pkg in "${BACKEND_PACKAGES[@]}"; do
    PACKAGE_ARGS+=(--package "$pkg")
  done
}

case "$SCOPE" in
  backend)
    build_backend_package_args
    RUN_ARGS=(
      "${PACKAGE_ARGS[@]}"
      --tests
    )
    REPORT_ARGS=("${PACKAGE_ARGS[@]}")
    ;;
  workspace)
    RUN_ARGS=(--workspace --tests --exclude kalam-pg-extension)
    REPORT_ARGS=(--workspace)
    ;;
  *)
    echo "Unsupported KALAMDB_COVERAGE_SCOPE='$SCOPE'. Use 'backend' or 'workspace'." >&2
    exit 1
    ;;
esac

if [[ "$INCLUDE_E2E" == "true" ]]; then
  if [[ "$SCOPE" == "backend" ]]; then
    RUN_ARGS+=(--features kalamdb-server/e2e-tests)
    REPORT_ARGS+=(--features kalamdb-server/e2e-tests)
  else
    RUN_ARGS+=(--features "kalam-cli/e2e-tests,kalamdb-server/e2e-tests")
    REPORT_ARGS+=(--features "kalam-cli/e2e-tests,kalamdb-server/e2e-tests")
  fi
  echo "Running coverage with e2e features enabled."
  echo "If tests require a server, start KalamDB first (e.g. cd backend && cargo run)."
fi

if ! cargo llvm-cov --version >/dev/null 2>&1; then
    cat >&2 <<'EOF'
cargo-llvm-cov is required for Rust coverage.
Install it with:
  cargo install cargo-llvm-cov --locked
Then add the LLVM tools component if it is missing:
  rustup component add llvm-tools-preview
EOF
    exit 1
fi

mkdir -p "$OUTPUT_DIR"

export RUST_TEST_THREADS="${KALAMDB_COVERAGE_RUST_TEST_THREADS:-1}"
TEST_JOBS="${KALAMDB_COVERAGE_TEST_JOBS:-2}"

cd "$ROOT_DIR"
cargo llvm-cov "${RUN_ARGS[@]}" \
  --jobs "$TEST_JOBS" \
  --lcov \
  --output-path "$LCOV_PATH"

cargo llvm-cov report \
  "${REPORT_ARGS[@]}" \
  --html \
  --output-dir "$HTML_DIR"

HTML_INDEX_PATH="$HTML_DIR/index.html"
if [[ ! -f "$HTML_INDEX_PATH" && -f "$HTML_DIR/html/index.html" ]]; then
  HTML_INDEX_PATH="$HTML_DIR/html/index.html"
fi

echo "Coverage report written to $LCOV_PATH"
echo "HTML report written to $HTML_INDEX_PATH"