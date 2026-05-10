#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
SERVER_URL="${KALAMDB_URL:-http://localhost:8080}"
SERVER_BIN="${KALAMDB_SERVER_BIN:-}"
WORK_DIR="${KALAMDB_SERVER_WORK_DIR:-}"
SERVER_LOG="${KALAMDB_SERVER_LOG:-}"
SERVER_PID_FILE="${KALAMDB_SERVER_PID_FILE:-}"
SERVER_TEMPLATE="${KALAMDB_SERVER_TEMPLATE:-$ROOT_DIR/backend/server.example.toml}"
JWT_SECRET="${KALAMDB_JWT_SECRET:-sdk-test-secret-key-minimum-32-characters-long}"
WAIT_SECONDS="${KALAMDB_SERVER_WAIT_SECONDS:-60}"

if [[ -z "$WORK_DIR" || -z "$SERVER_LOG" ]]; then
    echo "KALAMDB_SERVER_WORK_DIR and KALAMDB_SERVER_LOG are required." >&2
    exit 1
fi

if [[ -n "$SERVER_BIN" && ! -x "$SERVER_BIN" ]]; then
    chmod +x "$SERVER_BIN"
fi

healthcheck() {
    curl -sf "$SERVER_URL/health" > /dev/null 2>&1 \
        || curl -sf "$SERVER_URL/v1/api/healthcheck" > /dev/null 2>&1
}

rm -rf "$WORK_DIR"
mkdir -p "$WORK_DIR/data" "$WORK_DIR/logs" "$(dirname "$SERVER_LOG")"
cp "$SERVER_TEMPLATE" "$WORK_DIR/server.toml"

perl -0pi -e 's|data_path = "\./data"|data_path = "'"$WORK_DIR"'/data"|g; s|logs_path = "\./logs"|logs_path = "'"$WORK_DIR"'/logs"|g; s|jwt_secret = ".*"|jwt_secret = "'"$JWT_SECRET"'"|g' "$WORK_DIR/server.toml"

if [[ -n "$SERVER_BIN" ]]; then
    SERVER_CMD=("$SERVER_BIN" "$WORK_DIR/server.toml")
else
    SERVER_CMD=(cargo run --manifest-path "$ROOT_DIR/backend/Cargo.toml" --bin kalamdb-server -- "$WORK_DIR/server.toml")
fi

(
    cd "$ROOT_DIR"
    KALAMDB_SERVER_HOST=0.0.0.0 \
    KALAMDB_JWT_SECRET="$JWT_SECRET" \
    "${SERVER_CMD[@]}" > "$SERVER_LOG" 2>&1
) &
SERVER_PID=$!

if [[ -n "$SERVER_PID_FILE" ]]; then
    mkdir -p "$(dirname "$SERVER_PID_FILE")"
    printf '%s' "$SERVER_PID" > "$SERVER_PID_FILE"
fi

for ((i = 1; i <= WAIT_SECONDS; i++)); do
    if healthcheck; then
        echo "✅ SDK test server ready (${i}s)"
        if [[ -s "$SERVER_LOG" ]]; then
            echo "Recent SDK test server log output:"
            tail -n 40 "$SERVER_LOG" || true
        fi
        exit 0
    fi

    if ! kill -0 "$SERVER_PID" 2>/dev/null; then
        echo "❌ SDK test server died" >&2
        cat "$SERVER_LOG" || true
        exit 1
    fi

    echo "  Waiting for SDK test server... ($i/$WAIT_SECONDS)"
    sleep 1
done

echo "❌ Timed out waiting for SDK test server" >&2
cat "$SERVER_LOG" || true
kill "$SERVER_PID" 2>/dev/null || true
wait "$SERVER_PID" 2>/dev/null || true
exit 1