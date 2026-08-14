#!/usr/bin/env bash
# Run the KalamDB hot-only comparison benchmark.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${KALAMDB_SERVER_BIN:-$ROOT/bin/kalamdb-server}"
SETUP="$ROOT/setups/kalamdb"
RESULTS="$ROOT/results"
PORT="${KALAMDB_PORT:-2900}"
RPC_PORT="${KALAMDB_RPC_PORT:-$((PORT + 10))}"

mkdir -p "$RESULTS" "$SETUP/data" "$SETUP/logs"
[[ -x "$BIN" ]] || { echo "Missing $BIN — set KALAMDB_SERVER_BIN or run scripts/download-binaries.sh" >&2; exit 1; }

if curl -sf "http://127.0.0.1:${PORT}/health" >/dev/null 2>&1; then
  echo "Port ${PORT} already has a healthy KalamDB — refusing to clobber" >&2
  exit 1
fi

rm -rf "$SETUP/data" "$SETUP/logs"
mkdir -p "$SETUP/data" "$SETUP/logs"

cd "$SETUP"
KALAMDB_SERVER_PORT="$PORT" \
KALAMDB_CLUSTER_API_ADDR="127.0.0.1:$PORT" \
KALAMDB_CLUSTER_RPC_ADDR="127.0.0.1:$RPC_PORT" \
    "$BIN" "$SETUP/server.toml" >"$RESULTS/kalamdb-server.log" 2>&1 &
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
cargo build --release -p comparison_kalamdb
OUT="$RESULTS/kalamdb-$(date +%Y%m%d-%H%M%S).txt"
{
  echo "# KalamDB comparison (hot-only, tuned RocksDB/cache)"
  echo "# server_bin=${BIN}"
  echo "# flush.check_interval_seconds=0"
  echo "# table created without FLUSH_POLICY"
  echo "# rocksdb.block_cache=1GiB hot_data.write_buffer=128MiB"
  echo "# datafusion query_parallelism=2 max_partitions=1 batch_size=128"
  echo "# sql=parameterized; no multi-row batch (matches TB/PB one-row HTTP)"
  echo "# timed_read_response=bytes; HTTP status validation only (matches TB/PB)"
  echo "# http2_prior_knowledge=${KALAMDB_HTTP2:-0}"
  echo "# schema_response_cache=on; hot_only_pk_skip_cold=on"
  echo "# started=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  KALAMDB_URL="http://127.0.0.1:${PORT}" ./target/release/comparison_kalamdb
  echo "# finished=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} | tee "$OUT"

echo "Wrote $OUT"
