#!/usr/bin/env bash
# Download binaries (if needed) and run the requested systems sequentially.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REPO="$(cd "$ROOT/../.." && pwd)"

# Prefer a locally built release server (includes schema-cache / hot-only PK tweaks).
if [[ -z "${KALAMDB_SERVER_BIN:-}" ]]; then
  for candidate in \
    "$REPO/target/release/kalamdb-server" \
    "$ROOT/bin/kalamdb-server"
  do
    if [[ -x "$candidate" ]]; then
      export KALAMDB_SERVER_BIN="$candidate"
      break
    fi
  done
fi

"$ROOT/scripts/download-binaries.sh"

ORDER="${COMPARISON_ORDER:-kalamdb trailbase pocketbase}"
for system in $ORDER; do
  case "$system" in
    kalamdb) "$ROOT/scripts/run-kalamdb.sh" ;;
    trailbase) "$ROOT/scripts/run-trailbase.sh" ;;
    pocketbase) "$ROOT/scripts/run-pocketbase.sh" ;;
    *)
      echo "Unknown comparison system '$system' in COMPARISON_ORDER" >&2
      exit 2
      ;;
  esac
done
echo "All comparison runs finished. See $ROOT/results/"
