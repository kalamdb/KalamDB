#!/usr/bin/env bash
# Run KalamDB pgwire, KalamDB HTTP SQL, then PostgreSQL sequentially.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REPO="$(cd "$ROOT/../.." && pwd)"

if [[ -z "${KALAMDB_SERVER_BIN:-}" ]]; then
  for candidate in \
    "$REPO/target/release/kalamdb-server" \
    "$ROOT/../comparison/bin/kalamdb-server"
  do
    if [[ -x "$candidate" ]]; then
      export KALAMDB_SERVER_BIN="$candidate"
      break
    fi
  done
fi

ORDER="${COMPARISON_ORDER:-kalamdb kalamdb-http postgres}"
for system in $ORDER; do
  case "$system" in
    kalamdb) "$ROOT/scripts/run-kalamdb.sh" ;;
    kalamdb-http) "$ROOT/scripts/run-kalamdb-http.sh" ;;
    postgres) "$ROOT/scripts/run-postgres.sh" ;;
    *)
      echo "Unknown comparison system '$system' in COMPARISON_ORDER" >&2
      exit 2
      ;;
  esac
done
echo "All kalam-vs-pg runs finished. See $ROOT/results/"
