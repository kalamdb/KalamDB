#!/usr/bin/env bash
# test.sh - Build and test the Rust SDK packaging surface.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"

echo "==> cargo check (kalam-client)"
(cd "$REPO_ROOT/link/sdks/rust" && cargo check --features native-sdk,consumer,healthcheck)

echo "==> offline Rust SDK API tests"
(cd "$REPO_ROOT/link/sdks/rust" && cargo test --test offline_api --features consumer)

if [[ "${NO_SERVER:-}" == "true" ]]; then
  echo "Skipping server-backed tests because NO_SERVER=true"
  exit 0
fi

echo "==> integration Rust SDK tests (requires running server)"
(cd "$REPO_ROOT/link/sdks/rust" && cargo test --features e2e-tests -- --include-ignored)

echo "==> quickstart example"
(cd "$REPO_ROOT/link/sdks/rust/examples/quickstart" && cargo run -q)

echo "Rust SDK tests passed."
