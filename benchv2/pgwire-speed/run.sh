#!/usr/bin/env bash
# PGlite speed scripts, one simple query each, against PostgreSQL then KalamDB.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$ROOT/../.." && pwd)"
BACKEND="$REPO/backend"
SERVER_BIN="$REPO/target/release/kalamdb-server"
SQL_DIR="$ROOT/sql"
RESULTS="$ROOT/results"
LOGS="$ROOT/logs"
SQL_SOURCE="https://raw.githubusercontent.com/electric-sql/pglite/main/packages/benchmark/src"
PHASE_IDS=(1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16)

PG_PORT="${POSTGRES_PORT:-25435}"
PG_SETUP="$ROOT/setups/postgres"
PG_DATA="$PG_SETUP/data"
PG_LOGS="$PG_SETUP/logs"
PG_BINDIR=""
PG_STARTED=0

KALAM_HTTP_PORT=2921
KALAM_PG_PORT=25436
KALAM_SETUP="$ROOT/setups/kalamdb"
KALAM_PID=""
SERVER_BUILD_PID=""

ADMIN_USER="admin"
ADMIN_PASSWORD="kalamdb123"

usage() {
    echo "Usage: $0" >&2
    exit 2
}

if [[ $# -gt 0 ]]; then
    usage
fi

cleanup() {
    if [[ -n "$KALAM_PID" ]] && kill -0 "$KALAM_PID" 2>/dev/null; then
        kill "$KALAM_PID" 2>/dev/null || true
        wait "$KALAM_PID" 2>/dev/null || true
    fi
    if [[ "$PG_STARTED" -eq 1 && -n "$PG_BINDIR" ]]; then
        "$PG_BINDIR/pg_ctl" -D "$PG_DATA" -m fast stop >/dev/null 2>&1 || true
    fi
    if [[ -n "$SERVER_BUILD_PID" ]] && kill -0 "$SERVER_BUILD_PID" 2>/dev/null; then
        kill "$SERVER_BUILD_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT

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

resolve_pg_bindir() {
    if [[ -n "${POSTGRES_BIN:-}" ]]; then
        echo "$POSTGRES_BIN"
        return
    fi
    local prefix=""
    if command -v brew >/dev/null; then
        prefix="$(brew --prefix postgresql@18 2>/dev/null || true)"
        if [[ -x "${prefix:-}/bin/postgres" ]]; then
            echo "$prefix/bin"
            return
        fi
    fi
    for candidate in \
        /opt/homebrew/opt/postgresql@18/bin \
        /usr/local/opt/postgresql@18/bin \
        /usr/lib/postgresql/18/bin
    do
        if [[ -x "$candidate/postgres" ]]; then
            echo "$candidate"
            return
        fi
    done
    return 1
}

shared_buffers_mb() {
    local ram_bytes=""
    if ram_bytes=$(sysctl -n hw.memsize 2>/dev/null); then
        :
    elif ram_bytes=$(awk '/MemTotal/ {print $2 * 1024}' /proc/meminfo 2>/dev/null); then
        :
    else
        echo 256
        return
    fi
    local mb=$((ram_bytes / 8 / 1024 / 1024))
    if ((mb < 256)); then mb=256; fi
    if ((mb > 4096)); then mb=4096; fi
    echo "$mb"
}

port_in_use() {
    lsof -nP -iTCP:"$1" -sTCP:LISTEN >/dev/null 2>&1
}

start_server_build() {
    if [[ -x "$SERVER_BIN" ]] \
        && ! find "$BACKEND" "$REPO/Cargo.toml" "$REPO/Cargo.lock" \
            -newer "$SERVER_BIN" -print -quit 2>/dev/null | grep -q .; then
        echo "Using release server $SERVER_BIN"
        return
    fi
    echo "Building release KalamDB server"
    mkdir -p "$LOGS"
    (
        cd "$BACKEND"
        cargo build --release --bin kalamdb-server
    ) >"$LOGS/server-build.log" 2>&1 &
    SERVER_BUILD_PID=$!
}

wait_server_build() {
    if [[ -z "$SERVER_BUILD_PID" ]]; then
        return
    fi
    local pid="$SERVER_BUILD_PID"
    SERVER_BUILD_PID=""
    if ! wait "$pid"; then
        echo "Release server build failed. See $LOGS/server-build.log" >&2
        tail -n 40 "$LOGS/server-build.log" >&2 || true
        exit 1
    fi
    [[ -x "$SERVER_BIN" ]] || {
        echo "Build finished but $SERVER_BIN is missing" >&2
        exit 1
    }
}

ensure_homebrew_runtime_paths() {
    local prefix=""
    if command -v brew >/dev/null; then
        prefix="$(brew --prefix 2>/dev/null || true)"
    fi
    prefix="${prefix:-/opt/homebrew}"
    local keg share_src lib_src share_dst lib_dst
    keg="$(dirname "$PG_BINDIR")"
    share_src="$keg/share/postgresql"
    lib_src="$keg/lib/postgresql"
    share_dst="$prefix/share/postgresql@18"
    lib_dst="$prefix/lib/postgresql@18"
    if [[ -d "$share_src" && ! -e "$share_dst" ]]; then
        mkdir -p "$(dirname "$share_dst")"
        ln -s "$share_src" "$share_dst"
    fi
    if [[ -d "$lib_src" && ! -e "$lib_dst" ]]; then
        mkdir -p "$(dirname "$lib_dst")"
        ln -s "$lib_src" "$lib_dst"
    fi
}

start_postgres() {
    PG_BINDIR="$(resolve_pg_bindir)" || {
        echo "PostgreSQL 18 server binaries not found. Install postgresql@18 or set POSTGRES_BIN" >&2
        exit 1
    }
    ensure_homebrew_runtime_paths
    if port_in_use "$PG_PORT"; then
        echo "Port $PG_PORT is already in use. Set POSTGRES_PORT." >&2
        exit 1
    fi
    local share_dir
    share_dir="$("$PG_BINDIR/pg_config" --sharedir)"
    if [[ ! -f "$share_dir/postgres.bki" ]]; then
        for candidate in \
            "$(dirname "$PG_BINDIR")/share/postgresql" \
            "$(dirname "$PG_BINDIR")/share/postgresql@18"
        do
            if [[ -f "$candidate/postgres.bki" ]]; then
                share_dir="$candidate"
                break
            fi
        done
    fi
    [[ -f "$share_dir/postgres.bki" ]] || {
        echo "initdb input files not found under $share_dir" >&2
        exit 1
    }

    rm -rf "$PG_DATA" "$PG_LOGS"
    mkdir -p "$PG_DATA" "$PG_LOGS"
    "$PG_BINDIR/initdb" \
        --pgdata="$PG_DATA" \
        --username=postgres \
        --auth=trust \
        --encoding=UTF8 \
        --locale=C \
        --no-instructions \
        -L "$share_dir" \
        >/dev/null

    local shared_mb
    shared_mb="$(shared_buffers_mb)"
    "$PG_BINDIR/pg_ctl" -D "$PG_DATA" -l "$PG_LOGS/postgres.log" -o "\
-p ${PG_PORT} -c listen_addresses=127.0.0.1 -c unix_socket_directories=${PG_SETUP} \
-c shared_buffers=${shared_mb}MB -c effective_cache_size=$((shared_mb * 3))MB \
-c work_mem=4MB -c maintenance_work_mem=64MB \
-c synchronous_commit=off -c fsync=on -c full_page_writes=on \
-c max_connections=100 -c wal_compression=on \
-c checkpoint_timeout=15min -c max_wal_size=2GB" start >/dev/null
    PG_STARTED=1

    local _
    for _ in $(seq 1 60); do
        if "$PG_BINDIR/pg_isready" -h 127.0.0.1 -p "$PG_PORT" -U postgres >/dev/null 2>&1; then
            break
        fi
        sleep 0.25
    done
    "$PG_BINDIR/pg_isready" -h 127.0.0.1 -p "$PG_PORT" -U postgres >/dev/null
    "$PG_BINDIR/createdb" -h 127.0.0.1 -p "$PG_PORT" -U postgres bench
    echo "Started $("$PG_BINDIR/postgres" --version) on 127.0.0.1:${PG_PORT} (shared_buffers=${shared_mb}MB, synchronous_commit=off)"
}

stop_postgres() {
    if [[ "$PG_STARTED" -eq 1 && -n "$PG_BINDIR" ]]; then
        "$PG_BINDIR/pg_ctl" -D "$PG_DATA" -m fast stop >/dev/null
        PG_STARTED=0
    fi
}

bootstrap_admin() {
    local status_body
    status_body="$(curl --silent --fail "http://127.0.0.1:${KALAM_HTTP_PORT}/v1/api/auth/status")"
    if [[ "$status_body" != *'"needs_setup":true'* ]]; then
        return
    fi
    curl --silent --fail --show-error \
        -X POST "http://127.0.0.1:${KALAM_HTTP_PORT}/v1/api/auth/setup" \
        -H 'content-type: application/json' \
        -d "{\"user\":\"${ADMIN_USER}\",\"password\":\"${ADMIN_PASSWORD}\",\"root_password\":\"${ADMIN_PASSWORD}\"}" \
        >/dev/null
}

start_kalam() {
    if port_in_use "$KALAM_HTTP_PORT" || port_in_use "$KALAM_PG_PORT"; then
        echo "KalamDB port $KALAM_HTTP_PORT or $KALAM_PG_PORT is already in use." >&2
        exit 1
    fi
    rm -rf "$KALAM_SETUP/data" "$KALAM_SETUP/logs"
    mkdir -p "$KALAM_SETUP/data" "$KALAM_SETUP/logs" "$LOGS"
    echo "Starting one release KalamDB server (pgwire 127.0.0.1:${KALAM_PG_PORT})"
    (
        cd "$KALAM_SETUP"
        "$SERVER_BIN" "$KALAM_SETUP/server.toml"
    ) >"$LOGS/kalamdb-server.log" 2>&1 &
    KALAM_PID=$!

    local _
    for _ in $(seq 1 90); do
        if curl --silent --fail --output /dev/null "http://127.0.0.1:${KALAM_HTTP_PORT}/health"; then
            bootstrap_admin
            return
        fi
        if ! kill -0 "$KALAM_PID" 2>/dev/null; then
            echo "KalamDB exited before it became healthy" >&2
            tail -n 40 "$LOGS/kalamdb-server.log" >&2 || true
            exit 1
        fi
        sleep 1
    done
    echo "Timed out waiting for KalamDB health" >&2
    tail -n 40 "$LOGS/kalamdb-server.log" >&2 || true
    exit 1
}

stop_kalam() {
    if [[ -n "$KALAM_PID" ]] && kill -0 "$KALAM_PID" 2>/dev/null; then
        kill "$KALAM_PID" 2>/dev/null || true
        wait "$KALAM_PID" 2>/dev/null || true
    fi
    KALAM_PID=""
}

cd "$ROOT"
mkdir -p "$RESULTS" "$LOGS"
download_scripts
echo "Building pgwire-speed client"
cargo build --release --bin pgwire-speed
CLIENT="$ROOT/target/release/pgwire-speed"

start_server_build

echo "PostgreSQL"
start_postgres
"$CLIENT" run \
    "postgres://postgres@127.0.0.1:${PG_PORT}/bench" \
    "$SQL_DIR" \
    "$RESULTS/postgres.json" \
    postgres
stop_postgres

wait_server_build
start_kalam
echo "KalamDB"
"$CLIENT" run \
    "postgres://${ADMIN_USER}:${ADMIN_PASSWORD}@127.0.0.1:${KALAM_PG_PORT}/kalam" \
    "$SQL_DIR" \
    "$RESULTS/kalam.json" \
    kalamdb
stop_kalam

"$CLIENT" report "$RESULTS/postgres.json" "$RESULTS/kalam.json"
trap - EXIT
