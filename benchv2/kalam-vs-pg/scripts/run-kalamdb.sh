#!/usr/bin/env bash
# Run the KalamDB side of the pgwire comparison (hot-only + postgres_wire).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REPO="$(cd "$ROOT/../.." && pwd)"
BIN="${KALAMDB_SERVER_BIN:-}"
SETUP="$ROOT/setups/kalamdb"
RESULTS="$ROOT/results"
HTTP_PORT="${KALAMDB_PORT:-2901}"
RPC_PORT="${KALAMDB_RPC_PORT:-$((HTTP_PORT + 10))}"
PG_PORT="${KALAMDB_PGWIRE_PORT:-25432}"

if [[ -z "$BIN" ]]; then
  for candidate in \
    "$REPO/target/release/kalamdb-server" \
    "$ROOT/../comparison/bin/kalamdb-server"
  do
    if [[ -x "$candidate" ]]; then
      BIN="$candidate"
      break
    fi
  done
fi

mkdir -p "$RESULTS" "$SETUP/data" "$SETUP/logs"
[[ -n "$BIN" && -x "$BIN" ]] || {
  echo "Missing kalamdb-server — set KALAMDB_SERVER_BIN or cargo build -p kalamdb-server --release" >&2
  exit 1
}

if curl -sf "http://127.0.0.1:${HTTP_PORT}/health" >/dev/null 2>&1; then
  echo "Port ${HTTP_PORT} already has a healthy KalamDB — refusing to clobber" >&2
  exit 1
fi

rm -rf "$SETUP/data" "$SETUP/logs"
mkdir -p "$SETUP/data" "$SETUP/logs"

cd "$SETUP"
KALAMDB_SERVER_PORT="$HTTP_PORT" \
KALAMDB_CLUSTER_API_ADDR="127.0.0.1:$HTTP_PORT" \
KALAMDB_CLUSTER_RPC_ADDR="127.0.0.1:$RPC_PORT" \
KALAMDB_ENABLE_PGWIRE=true \
KALAMDB_PGWIRE_HOST="127.0.0.1" \
KALAMDB_PGWIRE_PORT="$PG_PORT" \
    "$BIN" "$SETUP/server.toml" >"$RESULTS/kalamdb-server.log" 2>&1 &
SERVER_PID=$!
cleanup() { kill "$SERVER_PID" 2>/dev/null || true; }
trap cleanup EXIT

for _ in $(seq 1 60); do
  if curl -sf "http://127.0.0.1:${HTTP_PORT}/health" >/dev/null; then
    break
  fi
  sleep 0.5
done
curl -sf "http://127.0.0.1:${HTTP_PORT}/health" >/dev/null

cd "$ROOT"
cargo build --release --bin kalam_vs_pg_kalamdb
OUT="$RESULTS/kalamdb-$(date +%Y%m%d-%H%M%S).txt"
{
  echo "# KalamDB vs PostgreSQL — KalamDB (pgwire, hot-only)"
  echo "# server_bin=${BIN}"
  echo "# http=127.0.0.1:${HTTP_PORT}"
  echo "# pgwire=127.0.0.1:${PG_PORT}"
  echo "# protocol=pgwire prepared statements; pool=16"
  echo "# flush.check_interval_seconds=0; table created without FLUSH_POLICY"
  echo "# rocksdb.memory_mode=server (cache≈RAM/8)"
  echo "# durability=sync_writes=false, WAL on (matches PG synchronous_commit=off)"
  echo "# started=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  KALAMDB_URL="http://127.0.0.1:${HTTP_PORT}" \
  KALAMDB_PG_URL="postgres://admin:kalamdb123@127.0.0.1:${PG_PORT}/kalam" \
    ./target/release/kalam_vs_pg_kalamdb
  echo "# finished=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} | tee "$OUT"

echo "Wrote $OUT"
