#!/usr/bin/env bash
# Run the SpaceTimeDB comparison benchmark (HTTP reducer + public SQL).
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
cleanup() { kill "$SERVER_PID" 2>/dev/null || true; }
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
OUT="$RESULTS/spacetimedb-$(date +%Y%m%d-%H%M%S).txt"
# shellcheck source=memory-sample.sh
source "$ROOT/scripts/memory-sample.sh"
memory_mark_ready "$SERVER_PID"
{
  echo "# SpaceTimeDB comparison (persistent websocket reducer + primary-key procedure)"
  echo "# write=reducer insert_message on a reused websocket"
  echo "# read=procedure get_message primary-key round trip (not a client cache, not HTTP SQL)"
  echo "# sockets=16 persistent"
  echo "# server_bin=${SERVER_BIN}"
  echo "# version=$(cat "$ROOT/bin/.spacetimedb-version" 2>/dev/null || echo unknown)"
  echo "# started=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  startup_header
  SPACETIMEDB_URL="$URL" ./target/release/comparison_spacetimedb
  echo "# finished=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} | tee "$OUT"

memory_append_result "$SERVER_PID" "$OUT"

echo "Wrote $OUT"
