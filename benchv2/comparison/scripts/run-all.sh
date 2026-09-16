#!/usr/bin/env bash
# Download binaries (if needed) and run the requested systems sequentially.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# Prefer a locally built release server (schema-first functions + hot-only PK tweaks).
# shellcheck source=resolve-kalamdb-server.sh
source "$ROOT/scripts/resolve-kalamdb-server.sh"

"$ROOT/scripts/download-binaries.sh"

ORDER="${COMPARISON_ORDER:-kalamdb kalamdb-functions trailbase pocketbase surrealdb}"
for system in $ORDER; do
  case "$system" in
    kalamdb) "$ROOT/scripts/run-kalamdb.sh" ;;
    kalamdb-functions) "$ROOT/scripts/run-kalamdb-functions.sh" ;;
    trailbase) "$ROOT/scripts/run-trailbase.sh" ;;
    pocketbase) "$ROOT/scripts/run-pocketbase.sh" ;;
    surrealdb) "$ROOT/scripts/run-surrealdb.sh" ;;
    *)
      echo "Unknown comparison system '$system' in COMPARISON_ORDER" >&2
      exit 2
      ;;
  esac
done
echo "All comparison runs finished. See $ROOT/results/"
