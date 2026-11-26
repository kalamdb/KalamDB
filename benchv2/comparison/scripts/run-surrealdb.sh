#!/usr/bin/env bash
# Run the SurrealDB comparison benchmark (HTTP REST /key API, RocksDB).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$ROOT/bin/surreal"
SETUP="$ROOT/setups/surrealdb"
RESULTS="$ROOT/results"
PORT="${SURREALDB_PORT:-8000}"

mkdir -p "$RESULTS" "$SETUP/data"
[[ -x "$BIN" ]] || { echo "Missing $BIN — run scripts/download-binaries.sh" >&2; exit 1; }

if curl -sf "http://127.0.0.1:${PORT}/health" >/dev/null 2>&1; then
  echo "Port ${PORT} already has SurrealDB — refusing" >&2
  exit 1
fi

rm -rf "$SETUP/data"
mkdir -p "$SETUP/data"

# shellcheck source=memory-sample.sh
source "$ROOT/scripts/memory-sample.sh"
startup_begin
"$BIN" start \
  --log error \
  --bind "127.0.0.1:${PORT}" \
  --user root \
  --pass root \
  "rocksdb://${SETUP}/data?sync=every" \
  >"$RESULTS/surrealdb-server.log" 2>&1 &
SERVER_PID=$!
cleanup() {
  kill "$SERVER_PID" 2>/dev/null || true
  wait "$SERVER_PID" 2>/dev/null || true
}
trap cleanup EXIT

for _ in $(seq 1 400); do
  if curl -sf --max-time 1 "http://127.0.0.1:${PORT}/health" >/dev/null; then
    break
  fi
  sleep 0.05
done
curl -sf "http://127.0.0.1:${PORT}/health" >/dev/null
startup_finish
startup_only_exit surrealdb
if [[ "${COMPARISON_WORKLOAD:-operations}" == "realtime" ]]; then
  COMPARISON_SERVER_BUILD="$(cat "$ROOT/bin/.surreal-version") RocksDB sync=every" \
    bash "$ROOT/scripts/run-realtime.sh" --client surrealdb "http://127.0.0.1:${PORT}"
  exit 0
fi

cd "$ROOT"
cargo build --release -p comparison_surrealdb
OUT="$RESULTS/verified-surrealdb-$(date +%Y%m%d-%H%M%S).txt"
# shellcheck source=memory-sample.sh
source "$ROOT/scripts/memory-sample.sh"
memory_mark_ready "$SERVER_PID"
{
  echo "# SurrealDB comparison (HTTP REST /key API, rocksdb)"
  echo "# auth=POST /signin + Bearer JWT (not Basic-per-request)"
  echo "# durability=RocksDB sync=every"
  echo "# server_bin=${BIN}"
  echo "# version=$(cat "$ROOT/bin/.surreal-version" 2>/dev/null || echo unknown)"
  echo "# started=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  startup_header
  SURREALDB_URL="http://127.0.0.1:${PORT}" ./target/release/comparison_surrealdb
  echo "# finished=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} 2>&1 | tee "$OUT"

memory_append_result "$SERVER_PID" "$OUT"

echo "Wrote $OUT"
