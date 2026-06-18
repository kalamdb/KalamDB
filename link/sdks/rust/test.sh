#!/usr/bin/env bash
# test.sh - Build and test the Rust SDK packaging surface.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"

echo "==> cargo check (kalam-client)"
(cd "$REPO_ROOT" && cargo check -p kalam-client --features native-sdk,consumer,healthcheck)

echo "==> offline Rust SDK API tests"
(cd "$REPO_ROOT" && cargo test -p kalam-client --test offline_api --features consumer)

if [[ "${NO_SERVER:-}" == "true" ]]; then
  echo "Skipping server-backed tests because NO_SERVER=true"
  exit 0
fi

echo "==> integration Rust SDK tests (requires running server)"
(cd "$REPO_ROOT" && cargo test -p kalam-client-e2e -- --include-ignored)

echo "==> quickstart example"
(cd "$SCRIPT_DIR/examples/quickstart" && cargo run -q)

echo "Rust SDK tests passed."
