#!/usr/bin/env bash
# Profile KalamDB pgwire with Jaeger (OTLP). Small sample — not a bake-off.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REPO="$(cd "$ROOT/../.." && pwd)"

cd "$REPO/docker/utils"
docker compose up -d jaeger

echo "Building kalamdb-server with traceability,otel (release)..."
cd "$REPO"
cargo build -p kalamdb-server --release --features traceability,otel

export KALAMDB_SERVER_BIN="${KALAMDB_SERVER_BIN:-$REPO/target/release/kalamdb-server}"
export KALAMDB_OTLP_ENABLED=true
export KALAMDB_OTLP_ENDPOINT="${KALAMDB_OTLP_ENDPOINT:-http://127.0.0.1:4317}"
export KALAMDB_OTLP_PROTOCOL="${KALAMDB_OTLP_PROTOCOL:-grpc}"
export KALAMDB_OTLP_SERVICE_NAME="${KALAMDB_OTLP_SERVICE_NAME:-kalamdb-server}"
export KALAM_VS_PG_N="${KALAM_VS_PG_N:-200}"
export KALAM_VS_PG_LATENCY_INSERTS="${KALAM_VS_PG_LATENCY_INSERTS:-100}"
export KALAM_VS_PG_READS="${KALAM_VS_PG_READS:-100}"

"$ROOT/scripts/run-kalamdb.sh"

echo
echo "Jaeger UI: http://127.0.0.1:16686  (service kalamdb-server, op sql.execute)"
echo "Summarize: python3 $ROOT/scripts/analyze-jaeger.py"
