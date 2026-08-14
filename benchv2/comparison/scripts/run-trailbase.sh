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

if curl -sf "http://127.0.0.1:${PORT}/" >/dev/null 2>&1 || curl -sf -o /dev/null "http://127.0.0.1:${PORT}/"; then
  # 404 on / is still "up"
  if curl -sf -o /dev/null -w '%{http_code}' "http://127.0.0.1:${PORT}/" | grep -Eq '200|404'; then
    echo "Port ${PORT} already in use — refuse to start second TrailBase" >&2
    exit 1
  fi
fi

(
  cd "$DEPOT"
  make clean >/dev/null 2>&1 || rm -rf data secrets backups uploads
)

DEPOT="$DEPOT" "$BIN" run --address "127.0.0.1:${PORT}" --stderr-logging >"$RESULTS/trailbase-server.log" 2>&1 &
SERVER_PID=$!
cleanup() { kill "$SERVER_PID" 2>/dev/null || true; }
trap cleanup EXIT

for _ in $(seq 1 60); do
  code=$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:${PORT}/" || true)
  if [[ "$code" == "404" || "$code" == "200" ]]; then
    break
  fi
  sleep 0.5
done

cd "$ROOT"
cargo build --release -p comparison_trailbase
OUT="$RESULTS/trailbase-$(date +%Y%m%d-%H%M%S).txt"
{
  echo "# TrailBase comparison (Record API)"
  echo "# started=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  TRAILBASE_URL="http://127.0.0.1:${PORT}" ./target/release/comparison_trailbase
  echo "# finished=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} | tee "$OUT"

echo "Wrote $OUT"
