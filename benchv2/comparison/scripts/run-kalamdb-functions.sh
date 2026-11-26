#!/usr/bin/env bash
# Run the KalamDB hot-only comparison benchmark over the functions REST API.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=resolve-kalamdb-server.sh
source "$ROOT/scripts/resolve-kalamdb-server.sh"
BIN="$KALAMDB_SERVER_BIN"
SETUP="$ROOT/setups/kalamdb"
RESULTS="$ROOT/results"
PORT="${KALAMDB_PORT:-2900}"
RPC_PORT="${KALAMDB_RPC_PORT:-$((PORT + 10))}"

mkdir -p "$RESULTS" "$SETUP/data" "$SETUP/logs"
[[ -x "$BIN" ]] || { echo "Missing $BIN — set KALAMDB_SERVER_BIN or cargo build --release -p kalamdb-server" >&2; exit 1; }

if curl -sf "http://127.0.0.1:${PORT}/health" >/dev/null 2>&1; then
  echo "Port ${PORT} already has a healthy KalamDB — refusing to clobber" >&2
  exit 1
fi

rm -rf "$SETUP/data" "$SETUP/logs"
mkdir -p "$SETUP/data" "$SETUP/logs"

cd "$SETUP"
# shellcheck source=memory-sample.sh
source "$ROOT/scripts/memory-sample.sh"
startup_begin
KALAMDB_SERVER_PORT="$PORT" \
KALAMDB_CLUSTER_API_ADDR="127.0.0.1:$PORT" \
KALAMDB_CLUSTER_RPC_ADDR="127.0.0.1:$RPC_PORT" \
    "$BIN" "$SETUP/server.toml" >"$RESULTS/kalamdb-functions-server.log" 2>&1 &
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
startup_only_exit kalamdb-functions

cd "$ROOT"
cargo build --release -p comparison_kalamdb_functions
OUT="$RESULTS/verified-kalamdb-functions-$(date +%Y%m%d-%H%M%S).txt"
# shellcheck source=memory-sample.sh
source "$ROOT/scripts/memory-sample.sh"
memory_mark_ready "$SERVER_PID"
{
  echo "# KalamDB comparison (hot-only, functions REST)"
  echo "# server_bin=${BIN}"
  echo "# flush.check_interval_seconds=0"
  echo "# table created without FLUSH_POLICY"
  echo "# rocksdb.memory_mode=compact (32MiB cache, 16MiB hot_data, 8MiB hot_index/raft, 1MiB system)"
  echo "# datafusion query_parallelism=2 max_partitions=1 batch_size=128"
  echo "# timed_path=POST /v1/functions/bench/{insert_message,get_message}"
  echo "# function_codec=${KALAMDB_FUNCTION_CODEC:-json}"
  echo "# setup_sql=CREATE TABLE + CREATE PROCEDURE only (not on the timed path)"
  echo "# nested_sql=ABI v2 ctx.db.execute/query(\$n, named input) (plan-cache binds)"
  echo "# durability=RocksDB synchronous WAL"
  echo "# http2_prior_knowledge=${KALAMDB_HTTP2:-0}"
  echo "# started=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  startup_header
  KALAMDB_URL="http://127.0.0.1:${PORT}" ./target/release/comparison_kalamdb_functions
  echo "# finished=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} 2>&1 | tee "$OUT"

memory_append_result "$SERVER_PID" "$OUT"

echo "Wrote $OUT"
