#!/usr/bin/env bash
# ==========================================================================
# pg/test.sh — pg_kalam end-to-end test runner (local dev)
# ==========================================================================
#
# Runs Rust e2e tests against a locally running pgrx PostgreSQL (port 28816)
# and a locally running KalamDB server (default http://127.0.0.1:8080).
# No Docker is started — both services must already be running.
#
# Usage (from repo root OR from pg/):
#   ./pg/test.sh
#   ./pg/test.sh --filter <pattern>   # nextest filter (default: test(e2e))
#   ./pg/test.sh --ddl                # include DDL tests (default)
#   ./pg/test.sh --no-ddl             # skip DDL tests and run the shorter DML/perf/scenario slice
#   ./pg/test.sh --no-fail-fast
#   ./pg/test.sh --help
#
# Prerequisites:
#   1. KalamDB server running:  cd backend && cargo run
#   2. pgrx installed locally:  cargo pgrx init --pg16 download
#
# This script will bootstrap the local pgrx postgres and install pg_kalam
# before running tests so the local test environment stays in sync.
#
# Environment variables (all optional):
#   KALAMDB_SERVER_URL    KalamDB HTTP base URL  (default: http://127.0.0.1:8080)
#   KALAMDB_GRPC_HOST     KalamDB gRPC host       (default: inferred from server URL)
#   KALAMDB_GRPC_PORT     KalamDB gRPC port       (default: inferred from server URL)
#   KALAMDB_PG_HOST       Postgres host           (default: 127.0.0.1)
#   KALAMDB_PG_PORT       Postgres port           (default: 28816)
#   KALAMDB_PG_USER       Postgres user           (default: $USER)
#
# NOTE: For CI, release.yml uses ./pg/docker/test.sh (a separate script).
# ==========================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# ── Defaults ──────────────────────────────────────────────────────────────
NEXTEST_FILTER="test(e2e)"
USE_NO_FAIL_FAST=false

run_nextest_slice() {
    local label="$1"
    shift

    step "$label"
    cargo nextest run \
        -p kalam-pg-extension \
        --features e2e \
        -E "$NEXTEST_FILTER" \
        --test-threads 1 \
        "$@" \
        "${NEXTEST_EXTRA_ARGS[@]+${NEXTEST_EXTRA_ARGS[@]}}"
}

# ── Argument parsing ───────────────────────────────────────────────────────
usage() { head -35 "$0"; }

while [[ $# -gt 0 ]]; do
    case "$1" in
        --filter)       NEXTEST_FILTER="$2"; shift ;;
        --ddl)          NEXTEST_FILTER="test(e2e)" ;;
        --no-ddl)       NEXTEST_FILTER="test(e2e) & !test(e2e_ddl)" ;;
        --no-fail-fast) USE_NO_FAIL_FAST=true ;;
        --fail-fast)    USE_NO_FAIL_FAST=false ;;
        -h|--help)      usage; exit 0 ;;
        *)              echo "Unknown option: $1"; exit 1 ;;
    esac
    shift
done

# ── Helpers ───────────────────────────────────────────────────────────────
step() { echo ""; echo "==> $*"; }
ok()   { echo "    OK: $*"; }
die()  { echo ""; echo "ERROR: $*" >&2; exit 1; }

infer_kalamdb_grpc_target() {
    local server_url="$1"
    local authority="${server_url#*://}"
    local host=""
    local http_port=""
    local grpc_port="9188"

    authority="${authority%%/*}"
    authority="${authority##*@}"

    if [[ "$authority" =~ ^\[([^]]+)\](:(.+))?$ ]]; then
        host="${BASH_REMATCH[1]}"
        http_port="${BASH_REMATCH[3]:-}"
    elif [[ "$authority" =~ ^([^:]+)(:([0-9]+))?$ ]]; then
        host="${BASH_REMATCH[1]}"
        http_port="${BASH_REMATCH[3]:-}"
    fi

    if [[ -z "$host" ]]; then
        host="127.0.0.1"
    fi

    case "$http_port" in
        8080) grpc_port="9188" ;;
        8081) grpc_port="9081" ;;
        8082) grpc_port="9082" ;;
        8083) grpc_port="9083" ;;
    esac

    printf '%s %s\n' "$host" "$grpc_port"
}

KALAMDB_SERVER_URL="${KALAMDB_SERVER_URL:-http://127.0.0.1:8080}"
read -r DEFAULT_KALAMDB_GRPC_HOST DEFAULT_KALAMDB_GRPC_PORT < <(infer_kalamdb_grpc_target "$KALAMDB_SERVER_URL")
KALAMDB_GRPC_HOST="${KALAMDB_GRPC_HOST:-$DEFAULT_KALAMDB_GRPC_HOST}"
KALAMDB_GRPC_PORT="${KALAMDB_GRPC_PORT:-$DEFAULT_KALAMDB_GRPC_PORT}"
export KALAMDB_SERVER_URL
export KALAMDB_GRPC_HOST
export KALAMDB_GRPC_PORT

# ── Print config ──────────────────────────────────────────────────────────
echo "========================================================"
echo " pg_kalam End-to-End Tests (local dev)"
echo "========================================================"
echo " KalamDB server:   ${KALAMDB_SERVER_URL}"
echo " KalamDB gRPC:     ${KALAMDB_GRPC_HOST}:${KALAMDB_GRPC_PORT}"
echo " Postgres:         ${KALAMDB_PG_HOST:-127.0.0.1}:${KALAMDB_PG_PORT:-28816} (pgrx)"
echo " Nextest filter:   $NEXTEST_FILTER"
echo "========================================================"

# ── Step 1: Check prerequisite tools ─────────────────────────────────────
step "Checking prerequisites ..."

if ! cargo nextest --version &>/dev/null 2>&1; then
    die "cargo-nextest is required. Install it with:
    cargo install cargo-nextest --locked"
fi
ok "cargo-nextest available"

# ── Step 2: Verify KalamDB is reachable ───────────────────────────────────
HEALTH_URL="${KALAMDB_SERVER_URL}/health"
if ! curl -sf --max-time 3 "$HEALTH_URL" >/dev/null 2>&1; then
    die "KalamDB server not reachable at ${KALAMDB_SERVER_URL}.
    Start the server first:
      cd backend && cargo run"
fi
ok "KalamDB server reachable at ${KALAMDB_SERVER_URL}"

# ── Step 3: Bootstrap local pgrx postgres ─────────────────────────────────
step "Bootstrapping local pgrx PostgreSQL and pg_kalam ..."
"$SCRIPT_DIR/scripts/pgrx-test-setup.sh"
ok "pgrx PostgreSQL is ready on ${KALAMDB_PG_HOST:-127.0.0.1}:${KALAMDB_PG_PORT:-28816} with pg_kalam installed"

# ── Step 4: Run e2e tests ─────────────────────────────────────────────────
step "Running e2e tests ..."
echo "    Filter:  $NEXTEST_FILTER"
echo ""

cd "$REPO_ROOT"

NEXTEST_EXTRA_ARGS=()
$USE_NO_FAIL_FAST && NEXTEST_EXTRA_ARGS+=("--no-fail-fast")

case "$NEXTEST_FILTER" in
    "test(e2e)")
        echo "    Running e2e_perf first to avoid warmed-server perf inflation."
        echo ""
        run_nextest_slice "Running perf e2e tests ..." --test e2e_perf
        run_nextest_slice \
            "Running remaining e2e tests ..." \
            --test e2e_ddl \
            --test e2e_dml \
            --test e2e_scenarios \
            --test extension_metadata \
            --test session_settings
        ;;
    "test(e2e) & !test(e2e_ddl)")
        echo "    Running e2e_perf first to avoid warmed-server perf inflation."
        echo ""
        run_nextest_slice "Running perf e2e tests ..." --test e2e_perf
        run_nextest_slice \
            "Running remaining non-DDL e2e tests ..." \
            --test e2e_dml \
            --test e2e_scenarios \
            --test extension_metadata \
            --test session_settings
        ;;
    *)
        run_nextest_slice "Running e2e tests ..."
        ;;
esac

echo ""
echo "========================================================"
echo " All e2e tests passed!"
echo "========================================================"
