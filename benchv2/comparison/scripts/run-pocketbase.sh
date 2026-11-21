#!/usr/bin/env bash
# Run the PocketBase comparison benchmark.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$ROOT/bin/pocketbase"
SETUP="$ROOT/setups/pocketbase"
RESULTS="$ROOT/results"
PORT=8090

mkdir -p "$RESULTS"
[[ -x "$BIN" ]] || { echo "Missing $BIN — run scripts/download-binaries.sh" >&2; exit 1; }

if curl -sf "http://127.0.0.1:${PORT}/api/health" >/dev/null 2>&1; then
  echo "Port ${PORT} already has PocketBase — refusing" >&2
  exit 1
fi

rm -rf "$SETUP/pb_data"
mkdir -p "$SETUP/pb_data"

# shellcheck source=memory-sample.sh
source "$ROOT/scripts/memory-sample.sh"
startup_begin
"$BIN" serve \
  --http="127.0.0.1:${PORT}" \
  --dir="$SETUP/pb_data" \
  --migrationsDir="$SETUP/pb_migrations" \
  --hooksDir="$SETUP/pb_hooks" \
  >"$RESULTS/pocketbase-server.log" 2>&1 &
SERVER_PID=$!
cleanup() { kill "$SERVER_PID" 2>/dev/null || true; }
trap cleanup EXIT

for _ in $(seq 1 400); do
  if curl -sf --max-time 1 "http://127.0.0.1:${PORT}/api/health" >/dev/null; then
    break
  fi
  sleep 0.05
done
startup_finish
startup_only_exit pocketbase

POCKETBASE_URL="http://127.0.0.1:${PORT}" "$ROOT/scripts/bootstrap-pocketbase.sh"

cd "$ROOT"
cargo build --release -p comparison_pocketbase
OUT="$RESULTS/pocketbase-$(date +%Y%m%d-%H%M%S).txt"
# shellcheck source=memory-sample.sh
source "$ROOT/scripts/memory-sample.sh"
memory_mark_ready "$SERVER_PID"
{
  echo "# PocketBase comparison (collections Record API)"
  echo "# server_bin=${BIN}"
  echo "# version=$(cat "$ROOT/bin/.pocketbase-version" 2>/dev/null || echo unknown)"
  echo "# started=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  startup_header
  POCKETBASE_URL="http://127.0.0.1:${PORT}" ./target/release/comparison_pocketbase
  echo "# finished=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} | tee "$OUT"

memory_append_result "$SERVER_PID" "$OUT"

echo "Wrote $OUT"
