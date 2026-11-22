#!/usr/bin/env bash
# Run the SpacetimeDB comparison benchmark (Rust SDK over WebSocket).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CLI="$ROOT/bin/spacetime"
SERVER_BIN="$ROOT/bin/spacetimedb-standalone"
MODULE="$ROOT/setups/spacetimedb/module"
DATA="$ROOT/setups/spacetimedb/data"
RESULTS="$ROOT/results"
PORT="${SPACETIMEDB_PORT:-3000}"
URL="http://127.0.0.1:${PORT}"

mkdir -p "$RESULTS"
[[ -x "$CLI" && -x "$SERVER_BIN" ]] || {
  echo "Missing SpaceTimeDB binaries — run scripts/download-binaries.sh" >&2
  exit 1
}

if curl -sf -o /dev/null -X POST "${URL}/v1/identity"; then
  echo "Port ${PORT} already has SpaceTimeDB — refusing" >&2
  exit 1
fi

rm -rf "$DATA"
mkdir -p "$DATA"

export CARGO_INCREMENTAL=0
# shellcheck source=memory-sample.sh
source "$ROOT/scripts/memory-sample.sh"
startup_begin
"$CLI" start \
  --listen-addr "127.0.0.1:${PORT}" \
  --data-dir "$DATA" \
  --non-interactive \
  >"$RESULTS/spacetimedb-server.log" 2>&1 &
SERVER_PID=$!
cleanup() {
  kill "$SERVER_PID" 2>/dev/null || true
  wait "$SERVER_PID" 2>/dev/null || true
}
trap cleanup EXIT

for _ in $(seq 1 400); do
  if curl -sf --max-time 1 -o /dev/null -X POST "${URL}/v1/identity"; then
    break
  fi
  sleep 0.05
done
curl -sf -o /dev/null -X POST "${URL}/v1/identity"
startup_finish
startup_only_exit spacetimedb

"$CLI" publish bench \
  --server "$URL" \
  --module-path "$MODULE" \
  --yes \
  --anonymous \
  --no-config

export CARGO_INCREMENTAL=0
"$CLI" generate \
  --lang rust \
  --module-path "$MODULE" \
  --out-dir "$ROOT/drivers/spacetimedb/src/module_bindings" \
  --yes \
  --no-config

cd "$ROOT"
cargo build --release -p comparison_spacetimedb
if [[ "${COMPARISON_WORKLOAD:-operations}" == "realtime" ]]; then
  COMPARISON_SERVER_BUILD="$(cat "$ROOT/bin/.spacetimedb-version") confirmed reads" \
    bash "$ROOT/scripts/run-realtime.sh" --client spacetimedb "$URL"
  exit 0
fi
OUT="$RESULTS/verified-spacetimedb-$(date +%Y%m%d-%H%M%S).txt"
# shellcheck source=memory-sample.sh
source "$ROOT/scripts/memory-sample.sh"
memory_mark_ready "$SERVER_PID"
{
  echo "# SpacetimeDB comparison (persistent websocket reducer + primary-key procedure)"
  echo "# write=reducer insert_message on a reused websocket"
  echo "# read=procedure get_message primary-key round trip (not a client cache, not HTTP SQL)"
  echo "# sockets=16 persistent"
  echo "# durability=file-backed commit log; confirmed reads enabled"
  echo "# server_bin=${SERVER_BIN}"
  echo "# version=$(cat "$ROOT/bin/.spacetimedb-version" 2>/dev/null || echo unknown)"
  echo "# started=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  startup_header
  SPACETIMEDB_URL="$URL" ./target/release/comparison_spacetimedb
  echo "# finished=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} 2>&1 | tee "$OUT"

memory_append_result "$SERVER_PID" "$OUT"

echo "Wrote $OUT"
