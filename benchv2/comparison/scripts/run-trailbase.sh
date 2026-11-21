#!/usr/bin/env bash
# Run the TrailBase comparison benchmark.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$ROOT/bin/trail"
DEPOT="$ROOT/setups/trailbase/traildepot"
RESULTS="$ROOT/results"
PORT=4000

mkdir -p "$RESULTS"
[[ -x "$BIN" ]] || { echo "Missing $BIN — run scripts/download-binaries.sh" >&2; exit 1; }

code=$(curl -s -o /dev/null -w '%{http_code}' --max-time 1 "http://127.0.0.1:${PORT}/" || true)
if [[ -n "$code" && "$code" != "000" ]]; then
  echo "Port ${PORT} already in use — refuse to start second TrailBase" >&2
  exit 1
fi

(
  cd "$DEPOT"
  make clean >/dev/null 2>&1 || rm -rf data secrets backups uploads
)

# shellcheck source=memory-sample.sh
source "$ROOT/scripts/memory-sample.sh"
startup_begin
DEPOT="$DEPOT" "$BIN" run --address "127.0.0.1:${PORT}" >"$RESULTS/trailbase-server.log" 2>&1 &
SERVER_PID=$!
cleanup() { kill "$SERVER_PID" 2>/dev/null || true; }
trap cleanup EXIT

code=000
for _ in $(seq 1 400); do
  code=$(curl -s -o /dev/null -w '%{http_code}' --max-time 1 "http://127.0.0.1:${PORT}/" || true)
  if [[ -n "$code" && "$code" != "000" ]]; then
    break
  fi
  sleep 0.05
done
if [[ -z "$code" || "$code" == "000" ]]; then
  echo "TrailBase did not accept connections on port ${PORT}" >&2
  exit 1
fi
startup_finish
startup_only_exit trailbase

cd "$ROOT"
cargo build --release -p comparison_trailbase
OUT="$RESULTS/trailbase-$(date +%Y%m%d-%H%M%S).txt"
# shellcheck source=memory-sample.sh
source "$ROOT/scripts/memory-sample.sh"
memory_mark_ready "$SERVER_PID"
{
  echo "# TrailBase comparison (Record API)"
  echo "# server_bin=${BIN}"
  echo "# version=$(cat "$ROOT/bin/.trail-version" 2>/dev/null || echo unknown)"
  echo "# stderr_logging=off (v0.33+ logs every request as JSON; do not enable for bake-off)"
  echo "# started=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  startup_header
  TRAILBASE_URL="http://127.0.0.1:${PORT}" ./target/release/comparison_trailbase
  echo "# finished=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} | tee "$OUT"

memory_append_result "$SERVER_PID" "$OUT"

echo "Wrote $OUT"
