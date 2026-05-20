#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
SERVER_URL="${KALAMDB_URL:-http://localhost:2900}"
SERVER_BIN="${KALAMDB_SERVER_BIN:-}"
WORK_DIR="${KALAMDB_SERVER_WORK_DIR:-}"
SERVER_LOG="${KALAMDB_SERVER_LOG:-}"
SERVER_PID_FILE="${KALAMDB_SERVER_PID_FILE:-}"
SERVER_TEMPLATE="${KALAMDB_SERVER_TEMPLATE:-$ROOT_DIR/backend/server.example.toml}"
JWT_SECRET="${KALAMDB_JWT_SECRET:-sdk-test-secret-key-minimum-32-characters-long}"
WAIT_SECONDS="${KALAMDB_SERVER_WAIT_SECONDS:-180}"
GRPC_HOST="${KALAMDB_GRPC_HOST:-}"
GRPC_PORT="${KALAMDB_GRPC_PORT:-}"
CLUSTER_ID="${KALAMDB_CLUSTER_ID:-sdk-test-cluster}"
CLUSTER_NODE_ID="${KALAMDB_CLUSTER_NODE_ID:-${KALAMDB_NODE_ID:-1}}"
CLUSTER_RPC_ADDR="${KALAMDB_CLUSTER_RPC_ADDR:-}"
CLUSTER_API_ADDR="${KALAMDB_CLUSTER_API_ADDR:-$SERVER_URL}"

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

server_port() {
    local authority="${SERVER_URL#*://}"
    authority="${authority%%/*}"

    if [[ "$authority" =~ ^\[(.*)\]:(.+)$ ]]; then
        printf '%s\n' "${BASH_REMATCH[2]}"
        return 0
    fi

    if [[ "$authority" == *:* ]]; then
        printf '%s\n' "${authority##*:}"
        return 0
    fi

    if [[ "$SERVER_URL" == https://* ]]; then
        printf '443\n'
    else
        printf '80\n'
    fi
}

server_host() {
    local authority="${SERVER_URL#*://}"
    authority="${authority%%/*}"

    if [[ "$authority" =~ ^\[(.*)\]:(.+)$ ]]; then
        printf '%s\n' "${BASH_REMATCH[1]}"
        return 0
    fi

    if [[ "$authority" == *:* ]]; then
        printf '%s\n' "${authority%%:*}"
        return 0
    fi

    printf '%s\n' "$authority"
}

port_in_use() {
    local port
    port="$(server_port)"
    lsof -tiTCP:"$port" -sTCP:LISTEN > /dev/null 2>&1
}

if healthcheck; then
    echo "❌ Refusing to reset $WORK_DIR: a server is already responding at $SERVER_URL" >&2
    exit 1
fi

if port_in_use; then
    echo "❌ Refusing to reset $WORK_DIR: target port $(server_port) is already in use" >&2
    lsof -nP -iTCP:"$(server_port)" -sTCP:LISTEN || true
    exit 1
fi

if [[ -z "$CLUSTER_RPC_ADDR" && -n "$GRPC_PORT" ]]; then
    if [[ -z "$GRPC_HOST" ]]; then
        GRPC_HOST="$(server_host)"
    fi
    CLUSTER_RPC_ADDR="${GRPC_HOST}:${GRPC_PORT}"
fi

rm -rf "$WORK_DIR"
mkdir -p "$WORK_DIR/data" "$WORK_DIR/logs" "$(dirname "$SERVER_LOG")"
cp "$SERVER_TEMPLATE" "$WORK_DIR/server.toml"

perl -0pi -e 's|data_path = "\./data"|data_path = "'"$WORK_DIR"'/data"|g; s|logs_path = "\./logs"|logs_path = "'"$WORK_DIR"'/logs"|g; s|jwt_secret = ".*"|jwt_secret = "'"$JWT_SECRET"'"|g; s|port = [0-9]+|port = '"$(server_port)"'|g' "$WORK_DIR/server.toml"

if [[ -n "$SERVER_BIN" ]]; then
    SERVER_CMD=("$SERVER_BIN" "$WORK_DIR/server.toml")
else
    SERVER_CMD=(cargo run --manifest-path "$ROOT_DIR/backend/Cargo.toml" --bin kalamdb-server -- "$WORK_DIR/server.toml")
fi

SERVER_ENV=(
    "KALAMDB_SERVER_HOST=0.0.0.0"
    "KALAMDB_JWT_SECRET=$JWT_SECRET"
)

if [[ -n "$CLUSTER_RPC_ADDR" ]]; then
    SERVER_ENV+=(
        "KALAMDB_CLUSTER_ID=$CLUSTER_ID"
        "KALAMDB_CLUSTER_NODE_ID=$CLUSTER_NODE_ID"
        "KALAMDB_CLUSTER_API_ADDR=$CLUSTER_API_ADDR"
        "KALAMDB_CLUSTER_RPC_ADDR=$CLUSTER_RPC_ADDR"
    )
fi

(
    cd "$ROOT_DIR"
    env "${SERVER_ENV[@]}" "${SERVER_CMD[@]}" > "$SERVER_LOG" 2>&1
) &
SERVER_PID=$!

if [[ -n "$SERVER_PID_FILE" ]]; then
    mkdir -p "$(dirname "$SERVER_PID_FILE")"
    printf '%s' "$SERVER_PID" > "$SERVER_PID_FILE"
fi

for ((i = 1; i <= WAIT_SECONDS; i++)); do
    if ! kill -0 "$SERVER_PID" 2>/dev/null; then
        echo "❌ SDK test server died" >&2
        cat "$SERVER_LOG" || true
        exit 1
    fi

    if healthcheck; then
        echo "✅ SDK test server ready (${i}s)"
        if [[ -s "$SERVER_LOG" ]]; then
            echo "Recent SDK test server log output:"
            tail -n 40 "$SERVER_LOG" || true
        fi
        exit 0
    fi

    echo "  Waiting for SDK test server... ($i/$WAIT_SECONDS)"
    sleep 1
done

echo "❌ Timed out waiting for SDK test server" >&2
cat "$SERVER_LOG" || true
kill "$SERVER_PID" 2>/dev/null || true
wait "$SERVER_PID" 2>/dev/null || true
exit 1