#!/usr/bin/env bash
set -euo pipefail

# Run the local Kalam CLI release binary from a separate project directory.
#
# Default layout expected when this script is run from a throwaway project dir:
#   parent/
#     kalamdb/cli/
#     my-test-project/
#       kalam-release-from-here.sh
#
# Override if your repo is elsewhere:
#   KALAMDB_CLI_DIR=/Users/jamal/git/KalamDB/cli ./kalam-release-from-here.sh init --yes ...

CLI_DIR="${KALAMDB_CLI_DIR:-../../cli}"
CLI_MANIFEST="${CLI_DIR%/}/Cargo.toml"

if [[ ! -f "$CLI_MANIFEST" ]]; then
  echo "Could not find Kalam CLI Cargo.toml at: $CLI_MANIFEST" >&2
  echo "Set KALAMDB_CLI_DIR to the repo cli directory, for example:" >&2
  echo "  KALAMDB_CLI_DIR=/Users/jamal/git/KalamDB/cli $0 <args>" >&2
  exit 1
fi

if [[ $# -eq 0 ]]; then
  cat >&2 <<'USAGE'
Usage:
  ./kalam-release-from-here.sh <kalam args>

Examples:
  ./kalam-release-from-here.sh init --yes --name test-app --schema-mode sql --server-mode local
  ./kalam-release-from-here.sh dev --force
  ./kalam-release-from-here.sh migration status
USAGE
  exit 2
fi

exec cargo run --release --manifest-path "$CLI_MANIFEST" -- "$@"
