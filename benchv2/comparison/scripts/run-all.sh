#!/usr/bin/env bash
# Download binaries (if needed) and run the requested systems sequentially.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export COMPARISON_WORKLOAD=operations

ORDER="${COMPARISON_ORDER:-kalamdb kalamdb-functions trailbase pocketbase surrealdb spacetimedb convex}"
for system in $ORDER; do
  case "$system" in
    kalamdb) "$ROOT/scripts/run-kalamdb.sh" ;;
    kalamdb-functions) "$ROOT/scripts/run-kalamdb-functions.sh" ;;
    trailbase) "$ROOT/scripts/run-trailbase.sh" ;;
    pocketbase) "$ROOT/scripts/run-pocketbase.sh" ;;
    surrealdb|spacetimedb)
      "$ROOT/scripts/download-binaries.sh" "$system"
      "$ROOT/scripts/run-$system.sh"
      ;;
    convex) "$ROOT/scripts/run-convex.sh" ;;
    *)
      echo "Unknown comparison system '$system' in COMPARISON_ORDER" >&2
      exit 2
      ;;
  esac
done
echo "All comparison runs finished. See $ROOT/results/"
