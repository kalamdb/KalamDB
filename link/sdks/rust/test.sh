#!/usr/bin/env bash
# test.sh - Build and test the Rust SDK packaging surface.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"

echo "==> cargo check (kalam-client)"
(cd "$REPO_ROOT/link/kalam-client" && cargo check --features native-sdk,consumer,healthcheck)

echo "==> cargo build (examples + tests workspace)"
(cd "$SCRIPT_DIR" && cargo build --workspace)

echo "==> offline Rust SDK API tests"
(cd "$SCRIPT_DIR" && cargo test --test offline_api)

if [[ "${NO_SERVER:-}" == "true" ]]; then
  echo "Skipping server-backed tests because NO_SERVER=true"
  exit 0
fi

echo "==> integration Rust SDK tests (requires running server)"
(cd "$SCRIPT_DIR" && cargo test -- --include-ignored)

echo "==> quickstart example"
(cd "$SCRIPT_DIR" && cargo run -q -p quickstart)

echo "Rust SDK tests passed."
