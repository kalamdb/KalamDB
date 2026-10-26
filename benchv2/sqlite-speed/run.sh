#!/usr/bin/env bash
# Same-machine SQLite speed comparison.
#
# Downloads the 16 PGlite scripts (tests 1-16, not 2.1 or 3.1), runs them in
# one local SQLite connection, then runs the same files against one release
# KalamDB server. The e2e suite is a different measurement: a dev-profile
# server compared with published times from another machine.
#
# Usage:
#   ./run.sh
#   ./run.sh --sqlite-only
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
BACKEND_DIR="$REPO_DIR/backend"
SERVER_BIN="$REPO_DIR/target/release/kalamdb-server"
SQL_DIR="$SCRIPT_DIR/sql"
RESULTS_DIR="$SCRIPT_DIR/results"
DATA_DIR="$SCRIPT_DIR/data/kalam"
LOG_FILE="$SCRIPT_DIR/logs/server.log"
BASE_URL="http://127.0.0.1:2920"
ROOT_PASSWORD="bench-local-pass"
SQL_SOURCE="https://raw.githubusercontent.com/electric-sql/pglite/main/packages/benchmark/src"
PHASE_IDS=(1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16)
SERVER_PID=""
SQLITE_ONLY="no"

usage() {
    echo "Usage: $0 [--sqlite-only]"
    exit 2
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --sqlite-only) SQLITE_ONLY="yes"; shift ;;
        -h|--help) usage ;;
        *) echo "Unknown argument: $1"; usage ;;
    esac
done

cleanup_server() {
    if [[ -n "$SERVER_PID" ]] && kill -0 "$SERVER_PID" 2>/dev/null; then
        kill "$SERVER_PID" 2>/dev/null || true
        wait "$SERVER_PID" 2>/dev/null || true
    fi
    SERVER_PID=""
}

download_scripts() {
    mkdir -p "$SQL_DIR"
    local id
    for id in "${PHASE_IDS[@]}"; do
        local path="$SQL_DIR/benchmark${id}.sql"
        if [[ -s "$path" ]]; then
            continue
        fi
        echo "Downloading benchmark${id}.sql"
        curl --fail --silent --show-error --location \
            "$SQL_SOURCE/benchmark${id}.sql" \
            --output "$path"
    done
}

ensure_release_server() {
    if [[ -x "$SERVER_BIN" ]] \
        && ! find "$BACKEND_DIR" "$REPO_DIR/Cargo.toml" "$REPO_DIR/Cargo.lock" \
            -newer "$SERVER_BIN" -print -quit 2>/dev/null | grep -q .; then
        echo "Using release server $SERVER_BIN"
        return
    fi
    echo "Building release KalamDB server"
    (
        cd "$BACKEND_DIR"
        cargo build --release --bin kalamdb-server
    )
}

wait_for_health() {
    local attempt
    for attempt in $(seq 1 90); do
        if curl --silent --fail --output /dev/null "$BASE_URL/health"; then
            return 0
        fi
        if [[ -n "$SERVER_PID" ]] && ! kill -0 "$SERVER_PID" 2>/dev/null; then
            echo "Release server exited before it became healthy"
            tail -n 40 "$LOG_FILE" 2>/dev/null || true
            return 1
        fi
        sleep 1
    done
    echo "Timed out waiting for $BASE_URL/health"
    tail -n 40 "$LOG_FILE" 2>/dev/null || true
    return 1
}

start_server() {
    if lsof -nP -iTCP:2920 -sTCP:LISTEN >/dev/null 2>&1; then
        echo "Port 2920 is already in use. Stop that process and rerun."
        lsof -nP -iTCP:2920 -sTCP:LISTEN || true
        return 1
    fi
    ensure_release_server
    rm -rf "$DATA_DIR"
    mkdir -p "$DATA_DIR" "$(dirname "$LOG_FILE")"
    : > "$LOG_FILE"
    echo "Starting one release server at $BASE_URL"
    KALAMDB_ROOT_PASSWORD="$ROOT_PASSWORD" \
        "$SERVER_BIN" "$SCRIPT_DIR/server.toml" >"$LOG_FILE" 2>&1 &
    SERVER_PID=$!
    trap cleanup_server EXIT
    wait_for_health
}

cd "$SCRIPT_DIR"
download_scripts
mkdir -p "$RESULTS_DIR" "$SCRIPT_DIR/data"

echo "SQLite $(python3 -c 'import sqlite3; print(sqlite3.sqlite_version)') on this machine"
python3 "$SCRIPT_DIR/compare.py" sqlite \
    --sql-dir "$SQL_DIR" \
    --disk "$SCRIPT_DIR/data/sqlite-disk.db" \
    --out "$RESULTS_DIR/sqlite.json"

if [[ "$SQLITE_ONLY" == "yes" ]]; then
    echo "SQLite timings are in $RESULTS_DIR/sqlite.json"
    exit 0
fi

start_server
python3 "$SCRIPT_DIR/compare.py" kalam \
    --sql-dir "$SQL_DIR" \
    --url "$BASE_URL" \
    --user root \
    --password "$ROOT_PASSWORD" \
    --out "$RESULTS_DIR/kalam.json"
python3 "$SCRIPT_DIR/compare.py" report \
    --sqlite "$RESULTS_DIR/sqlite.json" \
    --kalam "$RESULTS_DIR/kalam.json"
cleanup_server
trap - EXIT
