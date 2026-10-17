#!/usr/bin/env bash
# Soak a KalamDB server with checkout functions, topic consumers, and live queries.
#
#   ./run-soak.sh
#   ./run-soak.sh --seconds 60
#   ./run-soak.sh --minutes 30 --buyers 48 --subscribers 24
#   ./run-soak.sh --url http://127.0.0.1:2900 --server-pid 12345
#
# Loopback port 2900 is started from benchv2/server.toml when nothing is listening.
# An already running server is left alone and is not wiped.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$SCRIPT_DIR"

URL="${KALAMDB_URL:-http://127.0.0.1:2900}"
MINUTES="${KALAMDB_SOAK_MINUTES:-30}"
# Bash treats SECONDS as an elapsed-time counter. A variable with that name
# grows during the compile below and turns a 30-minute soak into a few seconds.
SOAK_SECONDS="${KALAMDB_SOAK_SECONDS:-0}"
BUYERS="${KALAMDB_SOAK_BUYERS:-48}"
SUBSCRIBERS="${KALAMDB_SOAK_SUBSCRIBERS:-24}"
DISPATCHERS="${KALAMDB_SOAK_DISPATCHERS:-8}"
USER_NAME="${KALAMDB_USER:-admin}"
PASSWORD="${KALAMDB_PASSWORD:-kalamdb123}"
SERVER_PID="${KALAMDB_SERVER_PID:-0}"
EXTRA=()

while [[ $# -gt 0 ]]; do
    case "$1" in
        --url) URL="$2"; shift 2 ;;
        --minutes) MINUTES="$2"; shift 2 ;;
        --seconds) SOAK_SECONDS="$2"; shift 2 ;;
        --buyers) BUYERS="$2"; shift 2 ;;
        --subscribers) SUBSCRIBERS="$2"; shift 2 ;;
        --dispatchers) DISPATCHERS="$2"; shift 2 ;;
        --user) USER_NAME="$2"; shift 2 ;;
        --password) PASSWORD="$2"; shift 2 ;;
        --server-pid) SERVER_PID="$2"; shift 2 ;;
        *) EXTRA+=("$1"); shift ;;
    esac
done

echo "Building kalamdb-soak"
cargo build --manifest-path "$SCRIPT_DIR/Cargo.toml" --bin kalamdb-soak

BIN="$SCRIPT_DIR/target/debug/kalamdb-soak"
MANAGED_PID=""

cleanup() {
    if [[ -n "$MANAGED_PID" ]] && kill -0 "$MANAGED_PID" 2>/dev/null; then
        kill "$MANAGED_PID" 2>/dev/null || true
        wait "$MANAGED_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT

port_from_url() {
    local trimmed="${1%/}"
    local maybe="${trimmed##*:}"
    if [[ "$maybe" =~ ^[0-9]+$ ]]; then
        echo "$maybe"
    else
        echo 80
    fi
}

if [[ "$URL" == http://127.0.0.1:* || "$URL" == http://localhost:* ]]; then
    PORT="$(port_from_url "$URL")"
    if ! curl -sf -o /dev/null --max-time 2 "$URL/health"; then
        SERVER_BIN="${KALAMDB_SERVER_BIN:-$REPO_DIR/target/debug/kalamdb-server}"
        if [[ ! -x "$SERVER_BIN" ]]; then
            SERVER_BIN="$REPO_DIR/target/release/kalamdb-server"
        fi
        if [[ ! -x "$SERVER_BIN" ]]; then
            echo "No server on $URL and no kalamdb-server binary. Build it with: cargo build -p kalamdb-server" >&2
            exit 1
        fi
        mkdir -p "$SCRIPT_DIR/logs"
        echo "Starting $SERVER_BIN on $URL"
        "$SERVER_BIN" "$SCRIPT_DIR/server.toml" >"$SCRIPT_DIR/logs/soak-server.log" 2>&1 &
        MANAGED_PID=$!
        SERVER_PID="$MANAGED_PID"
        for _ in $(seq 1 60); do
            if curl -sf -o /dev/null --max-time 2 "$URL/health"; then
                break
            fi
            if ! kill -0 "$MANAGED_PID" 2>/dev/null; then
                echo "Managed server exited. See $SCRIPT_DIR/logs/soak-server.log" >&2
                exit 1
            fi
            sleep 1
        done
    fi
fi

ARGS=(
    --url "$URL"
    --minutes "$MINUTES"
    --seconds "$SOAK_SECONDS"
    --buyers "$BUYERS"
    --subscribers "$SUBSCRIBERS"
    --dispatchers "$DISPATCHERS"
    --user "$USER_NAME"
    --password "$PASSWORD"
    --server-pid "$SERVER_PID"
)
if [[ ${#EXTRA[@]} -gt 0 ]]; then
    ARGS+=("${EXTRA[@]}")
fi

echo "Running soak against $URL for buyers=$BUYERS subscribers=$SUBSCRIBERS"
"$BIN" "${ARGS[@]}"
