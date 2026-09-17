#!/usr/bin/env bash
# Resolve KALAMDB_SERVER_BIN for the SQL and functions tracks.
# Prefer a locally built tree (schema-first functions), then the downloaded fallback.
# Source from run-kalamdb.sh / run-kalamdb-functions.sh / run-all.sh.

COMPARISON_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO_ROOT="$(cd "$COMPARISON_ROOT/../.." && pwd)"

if [[ -z "${KALAMDB_SERVER_BIN:-}" ]]; then
  for candidate in \
    "$REPO_ROOT/target/release/kalamdb-server" \
    "$COMPARISON_ROOT/bin/kalamdb-server"
  do
    if [[ -x "$candidate" ]]; then
      export KALAMDB_SERVER_BIN="$candidate"
      break
    fi
  done
fi

if [[ -z "${KALAMDB_SERVER_BIN:-}" || ! -x "$KALAMDB_SERVER_BIN" ]]; then
  echo "Building kalamdb-server --release for comparison SQL/functions tracks..." >&2
  cargo build --release -p kalamdb-server --manifest-path "$REPO_ROOT/Cargo.toml"
  export KALAMDB_SERVER_BIN="$REPO_ROOT/target/release/kalamdb-server"
fi

if [[ ! -x "${KALAMDB_SERVER_BIN:-}" ]]; then
  echo "Missing kalamdb-server — set KALAMDB_SERVER_BIN or cargo build --release -p kalamdb-server" >&2
  return 1 2>/dev/null || exit 1
fi
