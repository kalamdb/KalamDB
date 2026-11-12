#!/usr/bin/env bash
# PGlite speed scripts, one simple query each, against PostgreSQL then KalamDB.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$ROOT/../.." && pwd)"
BACKEND="$REPO/backend"
# dev is the repository default. Release is opt-in and is not the 039 baseline.
PROFILE="${KALAM_CARGO_PROFILE:-dev}"
case "$PROFILE" in
    dev)
        TARGET_DIR="debug"
        ;;
    release)
        TARGET_DIR="release"
        ;;
    *)
        echo "KALAM_CARGO_PROFILE must be dev or release" >&2
        exit 2
        ;;
esac
SERVER_BIN="$REPO/target/${TARGET_DIR}/kalamdb-server"
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
    local status=$?
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
    exit "$status"
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

build_server() {
    echo "Building ${PROFILE} KalamDB server"
    mkdir -p "$LOGS"
    (
        cd "$BACKEND"
        if [[ "$PROFILE" == "release" ]]; then
            cargo build --release --bin kalamdb-server
        else
            cargo build --bin kalamdb-server
        fi
    ) >"$LOGS/server-build.log" 2>&1 || {
        echo "Server build failed. See $LOGS/server-build.log" >&2
        tail -n 40 "$LOGS/server-build.log" >&2 || true
        exit 1
    }
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
    echo "Starting one ${PROFILE} KalamDB server (pgwire 127.0.0.1:${KALAM_PG_PORT})"
    (
        cd "$KALAM_SETUP"
        exec "$SERVER_BIN" "$KALAM_SETUP/server.toml"
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

write_metadata() {
    local dir="$1"
    local commit dirty_diff dirty_status toolchain sql_hashes server_hash client_hash
    commit="$(git -C "$REPO" rev-parse HEAD 2>/dev/null || echo unknown)"
    dirty_diff="$(git -C "$REPO" diff --binary | shasum -a 256 | awk '{print $1}')"
    dirty_status="$(git -C "$REPO" status --porcelain | shasum -a 256 | awk '{print $1}')"
    toolchain="$(rustc --version) / $(cargo --version)"
    sql_hashes="$(shasum -a 256 "$SQL_DIR"/benchmark*.sql)"
    server_hash="$(shasum -a 256 "$SERVER_BIN" | awk '{print $1}')"
    client_hash="$(shasum -a 256 "$CLIENT" | awk '{print $1}')"
    cat >"$dir/metadata.txt" <<EOF
timestamp_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)
commit=${commit}
dirty_diff_sha256=${dirty_diff}
dirty_status_sha256=${dirty_status}
toolchain=${toolchain}
profile=${PROFILE}
features=workspace default; kalamdb-server bin; pgwire-speed bin
server_sha256=${server_hash}
client_sha256=${client_hash}
connection=1
protocol=postgres simple query, one message per script
wal=enabled
fsync_wait=postgres synchronous_commit=off fsync=on; kalamdb sync_writes=false disable_wal=false
flush=hot-only check_interval_seconds=0
startup_and_auth=outside SQL timings
cache=each suite starts from empty postgres and kalam data directories
engine_order=odd suites postgres then kalamdb; even suites kalamdb then postgres
script_order=benchmark1.sql through benchmark16.sql, dependent, not reshuffled
dml_stage_trace=${KALAMDB_DML_STAGE_TRACE:-off}
sql_stage_trace=${KALAMDB_SQL_STAGE_TRACE:-off}
timing_note=leave both traces unset for timing suites; set KALAMDB_DML_STAGE_TRACE=1 for a separate profiling run
sql_sha256:
${sql_hashes}
EOF
}

run_engine() {
    local engine="$1"
    local dest="$2"
    if [[ "$engine" == "postgres" ]]; then
        echo "PostgreSQL"
        start_postgres
        "$CLIENT" run \
            "postgres://postgres@127.0.0.1:${PG_PORT}/bench" \
            "$SQL_DIR" \
            "$dest/postgres.json" \
            postgres
        stop_postgres
    else
        echo "KalamDB"
        start_kalam
        "$CLIENT" run \
            "postgres://${ADMIN_USER}:${ADMIN_PASSWORD}@127.0.0.1:${KALAM_PG_PORT}/kalam" \
            "$SQL_DIR" \
            "$dest/kalam.json" \
            kalamdb
        stop_kalam
    fi
}

run_pair() {
    local dest="$1"
    local postgres_first="$2"
    mkdir -p "$dest"
    if [[ "$postgres_first" == "1" ]]; then
        echo "order=postgres then kalamdb; data reset by each engine start"
        run_engine postgres "$dest"
        run_engine kalamdb "$dest"
    else
        echo "order=kalamdb then postgres; data reset by each engine start"
        run_engine kalamdb "$dest"
        run_engine postgres "$dest"
    fi
    "$CLIENT" report "$dest/postgres.json" "$dest/kalam.json"
}

summarize_recorded_suites() {
    local dir="$1"
    local count="$2"
    python3 - "$dir" "$count" <<'PY'
import json, sys
from pathlib import Path
root = Path(sys.argv[1])
count = int(sys.argv[2])
names = [f"{index} {label}" for index, label in [
    ("1", "1000 INSERTs"),
    ("2", "25000 INSERTs in a transaction"),
    ("3", "25000 INSERTs into an indexed table"),
    ("4", "100 SELECTs without an index"),
    ("5", "100 SELECTs on a string comparison"),
    ("6", "Creating an index"),
    ("7", "5000 SELECTs with an index"),
    ("8", "1000 UPDATEs without an index"),
    ("9", "25000 UPDATEs with an index"),
    ("10", "25000 text UPDATEs with an index"),
    ("11", "INSERTs from a SELECT"),
    ("12", "DELETE without an index"),
    ("13", "DELETE with an index"),
    ("14", "A big INSERT after a big DELETE"),
    ("15", "A big DELETE followed by many small INSERTs"),
    ("16", "DROP TABLE"),
]]
postgres = []
kalam = []
for suite in range(1, count + 1):
    postgres.append(json.loads((root / f"suite-{suite}" / "postgres.json").read_text())["seconds"])
    kalam.append(json.loads((root / f"suite-{suite}" / "kalam.json").read_text())["seconds"])

def median(values):
    ordered = sorted(values)
    return ordered[(len(ordered) - 1) // 2]

lines = ["profile recorded suites; seconds; median and range; not comparable to release timings", ""]
lines.append(f"{'#':<4} {'test':<46} {'pg median':>10} {'pg range':>16} {'kalam median':>12} {'kalam range':>16}")
pg_totals = [sum(sample) for sample in postgres]
kalam_totals = [sum(sample) for sample in kalam]
for index, name in enumerate(names):
    pg = [sample[index] for sample in postgres]
    ka = [sample[index] for sample in kalam]
    lines.append(
        f"{name:<51} {median(pg):10.3f} {min(pg):7.3f}..{max(pg):<7.3f} {median(ka):12.3f} {min(ka):7.3f}..{max(ka):<7.3f}"
    )
lines.append(
    f"{'total':<51} {median(pg_totals):10.3f} {min(pg_totals):7.3f}..{max(pg_totals):<7.3f} {median(kalam_totals):12.3f} {min(kalam_totals):7.3f}..{max(kalam_totals):<7.3f}"
)
text = "\n".join(lines) + "\n"
(root / "summary.txt").write_text(text)
print(text, end="")
PY
}

cd "$ROOT"
mkdir -p "$RESULTS" "$LOGS"
download_scripts
if [[ "${KALAM_BASELINE:-}" == "1" ]]; then
    WARMUP_SUITES=1
    RECORDED_SUITES=5
else
    WARMUP_SUITES="${KALAM_WARMUP_SUITES:-0}"
    RECORDED_SUITES="${KALAM_RECORDED_SUITES:-1}"
fi

echo "Building ${PROFILE} pgwire-speed client"
if [[ "$PROFILE" == "release" ]]; then
    cargo build --release --bin pgwire-speed
else
    cargo build --bin pgwire-speed
fi
CLIENT="$ROOT/target/${TARGET_DIR}/pgwire-speed"
build_server

RUN_STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
RUN_COMMIT="$(git -C "$REPO" rev-parse --short=12 HEAD 2>/dev/null || echo unknown)"
RUN_DIR="$RESULTS/runs/${RUN_STAMP}-${PROFILE}-${RUN_COMMIT}"
mkdir -p "$RUN_DIR"
write_metadata "$RUN_DIR"
echo "Results directory $RUN_DIR"

TOTAL=$((WARMUP_SUITES + RECORDED_SUITES))
for ((suite = 1; suite <= TOTAL; suite++)); do
    if ((suite <= WARMUP_SUITES)); then
        DEST="$RUN_DIR/warmup"
        LABEL="warmup (discarded)"
    else
        RECORDED=$((suite - WARMUP_SUITES))
        DEST="$RUN_DIR/suite-${RECORDED}"
        LABEL="recorded suite ${RECORDED}"
    fi
    if ((suite % 2 == 1)); then
        FIRST=1
    else
        FIRST=0
    fi
    echo "=== ${LABEL} ==="
    run_pair "$DEST" "$FIRST"
done

if ((RECORDED_SUITES > 1)); then
    summarize_recorded_suites "$RUN_DIR" "$RECORDED_SUITES"
fi
echo "Preserved any previous files under $RESULTS. This run is only in $RUN_DIR"
trap - EXIT
