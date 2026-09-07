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

"$BIN" start \
  --log error \
  --bind "127.0.0.1:${PORT}" \
  --user root \
  --pass root \
  "rocksdb://${SETUP}/data" \
  >"$RESULTS/surrealdb-server.log" 2>&1 &
SERVER_PID=$!
cleanup() { kill "$SERVER_PID" 2>/dev/null || true; }
trap cleanup EXIT

for _ in $(seq 1 60); do
  if curl -sf "http://127.0.0.1:${PORT}/health" >/dev/null; then
    break
  fi
  sleep 0.5
done
curl -sf "http://127.0.0.1:${PORT}/health" >/dev/null

cd "$ROOT"
cargo build --release -p comparison_surrealdb
OUT="$RESULTS/surrealdb-$(date +%Y%m%d-%H%M%S).txt"
{
  echo "# SurrealDB comparison (HTTP REST /key API, rocksdb)"
  echo "# auth=POST /signin + Bearer JWT (not Basic-per-request)"
  echo "# server_bin=${BIN}"
  echo "# started=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  SURREALDB_URL="http://127.0.0.1:${PORT}" ./target/release/comparison_surrealdb
  echo "# finished=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} | tee "$OUT"

echo "Wrote $OUT"
