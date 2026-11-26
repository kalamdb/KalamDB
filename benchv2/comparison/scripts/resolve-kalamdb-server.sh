#!/usr/bin/env bash
# Resolve KALAMDB_SERVER_BIN for the SQL and functions tracks.
# Use a current local build for schema-first functions and synchronous writes.
# Source from run-kalamdb.sh / run-kalamdb-functions.sh.

COMPARISON_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO_ROOT="$(cd "$COMPARISON_ROOT/../.." && pwd)"

if [[ -z "${KALAMDB_SERVER_BIN:-}" && ! -x "$REPO_ROOT/target/release/kalamdb-server" ]]; then
  echo "Building kalamdb-server --release for comparison SQL/functions tracks..." >&2
  cargo build --release -p kalamdb-server --features mimalloc --manifest-path "$REPO_ROOT/Cargo.toml"
fi
export KALAMDB_SERVER_BIN="${KALAMDB_SERVER_BIN:-$REPO_ROOT/target/release/kalamdb-server}"

if [[ ! -x "${KALAMDB_SERVER_BIN:-}" ]]; then
  echo "Missing kalamdb-server — set KALAMDB_SERVER_BIN or cargo build --release -p kalamdb-server" >&2
  return 1 2>/dev/null || exit 1
fi
