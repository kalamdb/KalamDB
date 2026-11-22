#!/usr/bin/env bash
# Run the Convex comparison benchmark (self-hosted HTTP query/mutation API).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$ROOT/bin/convex-local-backend-durable"
STAMP="$ROOT/bin/.convex-local-backend-durable-version"
APP="$ROOT/setups/convex"
DATA="$APP/data"
RESULTS="$ROOT/results"
PORT="${CONVEX_PORT:-3210}"
URL="http://127.0.0.1:${PORT}"
INSTANCE_NAME="convex-bench"

mkdir -p "$RESULTS"
[[ -x "$BIN" && -s "$STAMP" ]] || { echo "Run bash scripts/build-durable-sqlite.sh convex" >&2; exit 1; }
BUILD_STAMP="$(cat "$STAMP")"

if curl -sf -o /dev/null "${URL}/version"; then
  echo "Port ${PORT} already has Convex — refusing" >&2
  exit 1
fi

rm -rf "$DATA"
mkdir -p "$DATA"
SECRET="$(openssl rand -hex 32)"
ADMIN_KEY="$("$BIN" keygen admin-key --instance-name "$INSTANCE_NAME" --instance-secret "$SECRET")"

# shellcheck source=memory-sample.sh
source "$ROOT/scripts/memory-sample.sh"
startup_begin
(
  cd "$DATA"
  exec "$BIN" \
    --interface 127.0.0.1 \
    --port "$PORT" \
    --instance-name "$INSTANCE_NAME" \
    --instance-secret "$SECRET" \
    --disable-beacon \
    >"$RESULTS/convex-server.log" 2>&1
) &
SERVER_PID=$!
cleanup() {
  kill "$SERVER_PID" 2>/dev/null || true
  wait "$SERVER_PID" 2>/dev/null || true
}
trap cleanup EXIT

for _ in $(seq 1 400); do
  if curl -sf --max-time 1 -o /dev/null "${URL}/version"; then
    break
  fi
  sleep 0.05
done
curl -sf -o /dev/null "${URL}/version"
startup_finish
startup_only_exit convex

npm ci --prefix "$APP" --no-audit --no-fund

(
  cd "$APP"
  CONVEX_SELF_HOSTED_URL="$URL" \
  CONVEX_SELF_HOSTED_ADMIN_KEY="$ADMIN_KEY" \
    npx convex dev --once --typecheck disable
)
if [[ "${COMPARISON_WORKLOAD:-operations}" == "realtime" ]]; then
  COMPARISON_SERVER_BUILD="$BUILD_STAMP" bash "$ROOT/scripts/run-realtime.sh" --client convex "$URL"
  exit 0
fi

cd "$ROOT"
export CARGO_INCREMENTAL=0
cargo build --release -p comparison_convex
OUT="$RESULTS/verified-convex-$(date +%Y%m%d-%H%M%S).txt"
# shellcheck source=memory-sample.sh
source "$ROOT/scripts/memory-sample.sh"
memory_mark_ready "$SERVER_PID"
{
  echo "# Convex comparison (self-hosted HTTP /api/mutation and /api/query)"
  echo "# storage=SQLite rollback journal; synchronous=EXTRA"
  echo "# write=messages:insertMessage"
  echo "# read=messages:getMessage via db.get"
  echo "# auth=none (public functions)"
  echo "# server_bin=${BIN}"
  echo "# server_build=$BUILD_STAMP"
  echo "# started=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  startup_header
  CONVEX_URL="$URL" ./target/release/comparison_convex
  echo "# finished=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} 2>&1 | tee "$OUT"

memory_append_result "$SERVER_PID" "$OUT"

echo "Wrote $OUT"
