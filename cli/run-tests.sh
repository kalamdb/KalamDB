#!/usr/bin/env bash
# Helper script to run CLI tests with custom server URL and authentication
#
# Usage:
#   ./run-tests.sh                                    # Run all workspace tests + CLI e2e (default)
#   ./run-tests.sh --url http://localhost:3000        # Custom URL
#   ./run-tests.sh --password mypass                  # Custom password
#   ./run-tests.sh --url http://localhost:3000 --password mypass --test smoke
#
# Examples:
#   ./run-tests.sh --test smoke                       # Run smoke tests only
#   ./run-tests.sh --url http://localhost:3000        # Test on port 3000
#   ./run-tests.sh --test "smoke_test_core" --nocapture # Run specific test with output

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ENV_FILE="$SCRIPT_DIR/.env"

if [ -f "$ENV_FILE" ]; then
    set -a
    # shellcheck disable=SC1090
    source "$ENV_FILE"
    set +a
fi

# Default values
SERVER_URL="${KALAMDB_SERVER_URL:-}"
CLUSTER_URLS="${KALAMDB_CLUSTER_URLS:-}"
SERVER_TYPE="${KALAMDB_SERVER_TYPE:-}"
ROOT_PASSWORD="${KALAMDB_ROOT_PASSWORD-}"
ROOT_PASSWORD_SET=false
if [ "${KALAMDB_ROOT_PASSWORD+x}" = "x" ]; then
    ROOT_PASSWORD_SET=true
fi
TEST_JOBS="${KALAMDB_TEST_JOBS:-}"
TEST_JOBS_AUTO=false
TEST_FILTER=""
TEST_LIST_FILE=""
TEST_TARGET=""
NOCAPTURE=""
SHOW_HELP=false
PACKAGE_FILTERS=()

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        -u|--url)
            SERVER_URL="$2"
            shift 2
            ;;
        --cluster-urls|--urls)
            CLUSTER_URLS="$2"
            shift 2
            ;;
        --server-type)
            SERVER_TYPE="$2"
            shift 2
            ;;
        -j|--jobs)
            TEST_JOBS="$2"
            shift 2
            ;;
        -P|--package)
            PACKAGE_FILTERS+=("$2")
            shift 2
            ;;
        -p|--password)
            ROOT_PASSWORD="$2"
            ROOT_PASSWORD_SET=true
            shift 2
            ;;
        -t|--test)
            TEST_FILTER="$2"
            shift 2
            ;;
        --test-target)
            TEST_TARGET="$2"
            shift 2
            ;;
        --test-list)
            TEST_LIST_FILE="$2"
            shift 2
            ;;
        --nocapture)
            NOCAPTURE="--nocapture"
            shift
            ;;
        -h|--help)
            SHOW_HELP=true
            shift
            ;;
        *)
            echo "Unknown option: $1"
            SHOW_HELP=true
            shift
            ;;
    esac
done

if [ "$SHOW_HELP" = true ]; then
    echo "Usage: $0 [OPTIONS]"
    echo ""
    echo "Default: runs all workspace tests via cargo nextest, with CLI and backend e2e tests enabled"
    echo "         using features: kalam-cli/e2e-tests and kalamdb-server/e2e-tests."
    echo "         Untargeted full runs also execute"
    echo "         TypeScript SDK unit/browser/e2e, example, UI, and Dart test suites."
    echo ""
    echo "Options:"
    echo "  -u, --url <URL>          Single-node server URL"
    echo "  --cluster-urls <URLS>    Comma-separated cluster node URLs"
    echo "  --server-type <TYPE>     Server mode: fresh | running | cluster"
    echo "  -j, --jobs <N>           Override nextest process concurrency"
        echo "                           Cluster mode defaults to KALAMDB_CLUSTER_TEST_JOBS or 4"
    echo "  -P, --package <CRATE>    Limit the run to one package (repeatable)"
    echo "  -p, --password <PASS>    Root/admin password"
    echo "  -t, --test <FILTER>      Test filter (e.g., 'smoke', 'smoke_test_core')"
    echo "  --test-target <TARGET>   nextest test target/binary name (e.g., 'cluster')"
    echo "  --test-list <FILE|- >    Newline-delimited test filters to rerun one by one"
    echo "  --nocapture              Pass through test stdout/stderr (--no-capture)"
    echo "  -h, --help               Show this help message"
    echo ""
    echo "Examples:"
    echo "  $0 --test smoke --nocapture"
    echo "  $0 --url http://localhost:3000 --password mypass"
    echo "  $0 --cluster-urls http://127.0.0.1:2901,http://127.0.0.1:2902,http://127.0.0.1:2903 --server-type cluster"
    echo "  $0 --package kalam-cli --test-target cluster"
    echo "  $0 --package kalamdb-server --test-target test_scenarios_realtime"
    echo "  $0 --package kalam-cli --package kalam-link"
    echo "  $0 --test-list failed-tests.txt"
    exit 0
fi

if [ -n "$TEST_FILTER" ] && [ -n "$TEST_LIST_FILE" ]; then
    echo "Error: --test and --test-list cannot be used together."
    exit 1
fi

if [ -n "$TEST_LIST_FILE" ] && [ "$TEST_LIST_FILE" != "-" ] && [ ! -f "$TEST_LIST_FILE" ]; then
    echo "Error: test list file not found: $TEST_LIST_FILE"
    exit 1
fi

if [ -n "$CLUSTER_URLS" ]; then
    SERVER_TYPE="cluster"
fi

if [ -z "$SERVER_URL" ]; then
    if [ "$SERVER_TYPE" = "cluster" ] && [ -n "$CLUSTER_URLS" ]; then
        SERVER_URL="${CLUSTER_URLS%%,*}"
    else
        SERVER_URL="http://127.0.0.1:2900"
    fi
fi

AUTO_DETECTED_CLUSTER=false

detect_cluster_urls_from_health() {
    local base_url="$1"

    if [ -z "$base_url" ] || ! command -v python3 >/dev/null 2>&1; then
        return 1
    fi

    curl -fsS --max-time 2 "${base_url%/}/v1/api/cluster/health" 2>/dev/null | python3 - "$base_url" <<'PY'
import json
import sys
from urllib.parse import urlparse


def normalize_api_addr(api_addr: str, base_url: str) -> str:
    raw = api_addr.strip()
    if not raw:
        return ""

    base = urlparse(base_url.strip())
    base_scheme = base.scheme or "http"
    base_host = base.hostname or "127.0.0.1"

    parsed = urlparse(raw if "://" in raw else f"{base_scheme}://{raw}")
    host = parsed.hostname or ""
    if host in {"0.0.0.0", "::", "[::]"}:
        host = base_host

    if not host:
        return ""

    port = parsed.port
    if port is None:
        port = 443 if parsed.scheme == "https" else 80

    scheme = parsed.scheme or base_scheme
    return f"{scheme}://{host}:{port}"

try:
    payload = json.load(sys.stdin)
except Exception:
    raise SystemExit(1)

base_url = sys.argv[1].strip() if len(sys.argv) > 1 else ""

if not payload.get("is_cluster_mode"):
    raise SystemExit(1)

urls = []
for node in payload.get("nodes") or []:
    api_addr = normalize_api_addr(str(node.get("api_addr") or ""), base_url)
    if api_addr and api_addr not in urls:
        urls.append(api_addr)

if len(urls) <= 1:
    raise SystemExit(1)

print(",".join(urls))
PY
}

autodetect_cluster_mode() {
    local detected_cluster_urls

    if [ "$SERVER_TYPE" = "fresh" ]; then
        return 0
    fi

    detected_cluster_urls="$(detect_cluster_urls_from_health "$SERVER_URL")" || return 0

    if [ -z "$detected_cluster_urls" ]; then
        return 0
    fi

    CLUSTER_URLS="$detected_cluster_urls"
    SERVER_TYPE="cluster"
    AUTO_DETECTED_CLUSTER=true
}

autodetect_cluster_mode

if [ "$SERVER_TYPE" = "cluster" ] && [ -z "$TEST_JOBS" ]; then
    TEST_JOBS="${KALAMDB_CLUSTER_TEST_JOBS:-4}"
    TEST_JOBS_AUTO=true
fi

parse_host_port_from_url() {
    local url="$1"

    if ! command -v python3 >/dev/null 2>&1; then
        return 1
    fi

    python3 - "$url" <<'PY'
from urllib.parse import urlparse
import sys

url = sys.argv[1].strip()
if not url:
    raise SystemExit(1)

parsed = urlparse(url)
host = parsed.hostname
port = parsed.port
if not host:
    raise SystemExit(1)
if port is None:
    port = 443 if parsed.scheme == "https" else 80

print(f"{host}\n{port}")
PY
}

is_local_host() {
    case "$1" in
        127.0.0.1|localhost|::1|0.0.0.0)
            return 0
            ;;
        *)
            return 1
            ;;
    esac
}

validate_single_local_listener() {
    local host_port
    local host
    local port
    local pids
    local count

    if ! command -v lsof >/dev/null 2>&1; then
        return 0
    fi

    host_port="$(parse_host_port_from_url "$SERVER_URL")" || return 0
    host="$(printf '%s\n' "$host_port" | sed -n '1p')"
    port="$(printf '%s\n' "$host_port" | sed -n '2p')"

    if ! is_local_host "$host"; then
        return 0
    fi

    pids="$(lsof -n -P -iTCP:"$port" -sTCP:LISTEN -t 2>/dev/null | sort -u)"
    count="$(printf '%s\n' "$pids" | sed '/^$/d' | wc -l | tr -d ' ')"

    if [ "$count" -le 1 ]; then
        return 0
    fi

    echo "Error: multiple processes are listening on ${host}:${port}."
    echo "Running-server mode requires a single deterministic target."
    echo ""
    lsof -n -P -iTCP:"$port" -sTCP:LISTEN | head -n 20
    echo ""
    echo "Stop the extra server(s) and rerun ./run-tests.sh."
    exit 1
}

validate_cluster_health() {
    local health_url="$1"
    local summary

    if ! command -v python3 >/dev/null 2>&1; then
        return 0
    fi

    summary="$(python3 - "$health_url" "${CLUSTER_URLS:-}" <<'PY'
import json
import sys
from urllib.error import URLError
from urllib.parse import urlparse
from urllib.request import urlopen

target_url = sys.argv[1].rstrip("/")
cluster_urls_arg = sys.argv[2].strip() if len(sys.argv) > 2 else ""


def normalize_cluster_url(raw_url, fallback_url):
    raw = raw_url.strip()
    if not raw:
        return ""

    fallback = urlparse(fallback_url)
    fallback_scheme = fallback.scheme or "http"
    fallback_host = fallback.hostname or "127.0.0.1"

    parsed = urlparse(raw if "://" in raw else f"{fallback_scheme}://{raw}")
    host = parsed.hostname or ""
    if host in {"0.0.0.0", "::", "[::]"}:
        host = fallback_host

    if not host:
        return ""

    port = parsed.port
    if port is None:
        port = 443 if parsed.scheme == "https" else 80

    scheme = parsed.scheme or fallback_scheme
    return f"{scheme}://{host}:{port}"


def fetch_health(base_url):
    with urlopen(f"{base_url.rstrip('/')}/v1/api/cluster/health", timeout=3) as response:
        return json.load(response)


try:
    target_payload = fetch_health(target_url)
except Exception:
    print("ok")
    raise SystemExit(0)

if not target_payload.get("is_cluster_mode"):
    print("ok")
    raise SystemExit(0)

cluster_urls = [url.strip() for url in cluster_urls_arg.split(",") if url.strip()]
cluster_urls = [normalized for url in cluster_urls if (normalized := normalize_cluster_url(url, target_url))]
if not cluster_urls:
    seen = set()
    for node in target_payload.get("nodes") or []:
        api_addr = normalize_cluster_url(str(node.get("api_addr") or ""), target_url)
        if api_addr and api_addr not in seen:
            seen.add(api_addr)
            cluster_urls.append(api_addr)

if not cluster_urls:
    cluster_urls = [target_url]

payloads = []
failed_urls = []
for url in cluster_urls:
    try:
        payload = fetch_health(url)
    except (OSError, URLError, TimeoutError, json.JSONDecodeError):
        failed_urls.append(url)
        continue
    except Exception:
        failed_urls.append(url)
        continue
    if payload.get("is_cluster_mode"):
        payloads.append((url, payload))

if failed_urls:
    print(f"unreachable {','.join(failed_urls)}")
    raise SystemExit(0)

if not payloads:
    print("ok")
    raise SystemExit(0)

total_groups = max(int(payload.get("total_groups") or 0) for _, payload in payloads)
if total_groups <= 0:
    print("ok")
    raise SystemExit(0)

groups_leading = 0
seen_nodes = set()
for url, payload in payloads:
    node_id = payload.get("node_id") or url
    if node_id in seen_nodes:
        continue
    seen_nodes.add(node_id)
    groups_leading += int(payload.get("groups_leading") or 0)

if len(payloads) == 1:
    node_counts = [
        int(node.get("groups_leading") or 0)
        for node in (payloads[0][1].get("nodes") or [])
        if node.get("groups_leading") is not None
    ]
    if node_counts:
        groups_leading = sum(node_counts)

if groups_leading != total_groups:
    print(f"degraded {groups_leading} {total_groups}")
else:
    print("ok")
PY
)" || true

    case "$summary" in
        ok|"")
            return 0
            ;;
        unreachable\ *)
            set -- $summary
            echo "Error: configured cluster node(s) are unreachable: ${2}"
            echo "CLI e2e tests require every configured cluster node to be reachable."
            echo ""
            echo "Check: ${health_url%/}/v1/api/cluster/health"
            echo "Fix the running cluster state, then rerun ./run-tests.sh."
            exit 1
            ;;
        degraded\ *)
            set -- $summary
            echo "Error: cluster reports incomplete Raft group leadership (${2}/${3} groups leading across configured nodes)."
            echo "This usually means stale or mismatched local Raft state, and CLI e2e tests will fail nondeterministically."
            echo ""
            echo "Check: ${health_url%/}/v1/api/cluster/health"
            echo "Fix the running server state, then rerun ./run-tests.sh."
            exit 1
            ;;
    esac
}

preflight_running_server() {
    if [ "$SERVER_TYPE" = "fresh" ]; then
        return 0
    fi

    validate_single_local_listener
    validate_cluster_health "$SERVER_URL"
}

if [ ${#PACKAGE_FILTERS[@]} -gt 1 ]; then
    for package in "${PACKAGE_FILTERS[@]}"; do
        if [ "$package" = "kalam-cli" ]; then
            echo "Error: run kalam-cli separately when using --package because e2e-tests is package-specific."
            exit 1
        fi
    done
fi

FEATURE_MODE="workspace + CLI/backend e2e features"
if [ ${#PACKAGE_FILTERS[@]} -gt 0 ]; then
    if [ ${#PACKAGE_FILTERS[@]} -eq 1 ] && [ "${PACKAGE_FILTERS[0]}" = "kalam-cli" ]; then
        FEATURE_MODE="package + CLI e2e feature"
    elif [ ${#PACKAGE_FILTERS[@]} -eq 1 ] && [ "${PACKAGE_FILTERS[0]}" = "kalamdb-server" ]; then
        FEATURE_MODE="package + backend e2e feature"
    else
        FEATURE_MODE="package only"
    fi
fi

RUN_SUPPLEMENTARY_SUITES=false
SUPPLEMENTARY_MODE="skipped (targeted/package-scoped Rust run)"
if [ -z "$TEST_FILTER" ] \
    && [ -z "$TEST_LIST_FILE" ] \
    && [ -z "$TEST_TARGET" ] \
    && [ ${#PACKAGE_FILTERS[@]} -eq 0 ]; then
    RUN_SUPPLEMENTARY_SUITES=true
    SUPPLEMENTARY_MODE="TypeScript SDK unit/browser/e2e, examples, UI, and Dart"
fi

# Display configuration
echo "================================================"
echo "Running KalamDB Tests (cargo nextest)"
echo "================================================"
if [ -f "$ENV_FILE" ]; then
    echo "Env File:        $ENV_FILE"
else
    echo "Env File:        (none)"
fi
echo "Server Type:     ${SERVER_TYPE:-auto}"
if [ "$SERVER_TYPE" = "cluster" ]; then
    echo "Cluster URLs:    ${CLUSTER_URLS:-$SERVER_URL}"
    echo "Primary URL:     $SERVER_URL"
    if [ "$AUTO_DETECTED_CLUSTER" = true ]; then
        echo "Cluster Detect:  /v1/api/cluster/health"
    fi
else
    echo "Server URL:      $SERVER_URL"
fi
if [ ${#PACKAGE_FILTERS[@]} -gt 0 ]; then
    echo "Packages:        ${PACKAGE_FILTERS[*]}"
else
    echo "Packages:        workspace"
fi
echo "Root Password:   $([ -z "$ROOT_PASSWORD" ] && echo '(empty)' || echo '***')"
if [ -n "$TEST_TARGET" ]; then
    echo "Test Target:     $TEST_TARGET"
fi
echo "Test Filter:     $([ -z "$TEST_FILTER" ] && echo '(all tests)' || echo "$TEST_FILTER")"
if [ -n "$TEST_LIST_FILE" ]; then
    echo "Test List:       $TEST_LIST_FILE"
fi
if [ -n "$TEST_JOBS" ]; then
    if [ "$TEST_JOBS_AUTO" = true ]; then
        echo "Jobs:            $TEST_JOBS (cluster default)"
    else
        echo "Jobs:            $TEST_JOBS"
    fi
fi
echo "Mode:            $FEATURE_MODE"
echo "Supplementary:   $SUPPLEMENTARY_MODE"
echo "================================================"
echo ""

preflight_running_server

# Clear shared JWT caches so a restarted running server/cluster does not reuse
# stale admin/root tokens from a previous test session.
rm -f "${TMPDIR:-/tmp}/kalamdb_test_tokens.json" "${TMPDIR:-/tmp}/kalamdb_test_tokens.lock"

# Export environment variables
export KALAMDB_SERVER_URL="$SERVER_URL"
if [ -n "$CLUSTER_URLS" ]; then
    export KALAMDB_CLUSTER_URLS="$CLUSTER_URLS"
else
    unset KALAMDB_CLUSTER_URLS
fi

if [ -n "$SERVER_TYPE" ]; then
    export KALAMDB_SERVER_TYPE="$SERVER_TYPE"
else
    unset KALAMDB_SERVER_TYPE
fi

if [ "$ROOT_PASSWORD_SET" = true ]; then
    export KALAMDB_ROOT_PASSWORD="$ROOT_PASSWORD"
else
    unset KALAMDB_ROOT_PASSWORD
fi

TEST_ROOT_PASSWORD="${ROOT_PASSWORD:-kalamdb123}"
export KALAMDB_ADMIN_USER="${KALAMDB_ADMIN_USER:-admin}"
export KALAMDB_ADMIN_PASSWORD="${KALAMDB_ADMIN_PASSWORD:-$TEST_ROOT_PASSWORD}"
export KALAMDB_URL="${KALAMDB_URL:-$KALAMDB_SERVER_URL}"
export KALAMDB_USER="${KALAMDB_USER:-root}"
export KALAMDB_PASSWORD="${KALAMDB_PASSWORD:-$TEST_ROOT_PASSWORD}"
export KALAMDB_TEST_URL="${KALAMDB_TEST_URL:-$KALAMDB_SERVER_URL}"
export KALAMDB_TEST_USER="${KALAMDB_TEST_USER:-$KALAMDB_ADMIN_USER}"
export KALAMDB_TEST_PASSWORD="${KALAMDB_TEST_PASSWORD:-$KALAMDB_ADMIN_PASSWORD}"
export KALAM_URL="${KALAM_URL:-$KALAMDB_URL}"
export KALAM_USER="${KALAM_USER:-$KALAMDB_USER}"
export KALAM_PASS="${KALAM_PASS:-$KALAMDB_PASSWORD}"

# Ensure nextest is available
if ! cargo nextest --version >/dev/null 2>&1; then
    echo "Error: cargo-nextest is not installed."
    echo "Install it with: cargo install cargo-nextest"
    exit 1
fi

step() {
    echo ""
    echo "==> $*"
}

require_cmd() {
    command -v "$1" >/dev/null 2>&1 || {
        echo "Error: missing required command '$1'"
        exit 1
    }
}

package_filters_include() {
    local expected="$1"
    local package

    for package in "${PACKAGE_FILTERS[@]}"; do
        if [ "$package" = "$expected" ]; then
            return 0
        fi
    done

    return 1
}

should_start_dex_for_oidc_tests() {
    if [ "${KALAMDB_SKIP_DOCKER_DEX:-false}" = "true" ]; then
        return 1
    fi

    case "$TEST_TARGET:$TEST_FILTER" in
        *auth*|*oidc*|*OIDC*|*dex*|*Dex*)
            return 0
            ;;
    esac

    if [ -z "$TEST_FILTER" ] && [ -z "$TEST_TARGET" ]; then
        if [ ${#PACKAGE_FILTERS[@]} -eq 0 ] || package_filters_include "kalam-cli"; then
            return 0
        fi
    fi

    return 1
}

test_list_includes_cli_oidc() {
    if [ -z "$TEST_LIST_FILE" ]; then
        return 1
    fi

    if [ "$TEST_LIST_FILE" = "-" ]; then
        return 0
    fi

    grep -q 'oidc_cli_' "$TEST_LIST_FILE"
}

should_prebuild_cli_oidc_server_binary() {
    if [ ${#PACKAGE_FILTERS[@]} -gt 0 ] && ! package_filters_include "kalam-cli"; then
        return 1
    fi

    if [ -n "$TEST_LIST_FILE" ]; then
        test_list_includes_cli_oidc
        return $?
    fi

    if [ -n "$TEST_FILTER" ]; then
        [[ "$TEST_FILTER" == *oidc_cli_* ]]
        return $?
    fi

    if [ -n "$TEST_TARGET" ]; then
        [ "$TEST_TARGET" = "auth" ]
        return $?
    fi

    return 0
}

prebuild_cli_oidc_server_binary_if_needed() {
    if ! should_prebuild_cli_oidc_server_binary; then
        return 0
    fi

    step "Prebuilding kalamdb-server for CLI OIDC tests"
    (
        cd "$REPO_ROOT"
        cargo build -p kalamdb-server --bin kalamdb-server
    )
    export KALAMDB_SERVER_BIN="$REPO_ROOT/target/debug/kalamdb-server"
}

ensure_dex_for_oidc_tests() {
    if ! should_start_dex_for_oidc_tests; then
        return 0
    fi

    if ! command -v docker >/dev/null 2>&1; then
        echo "Warning: Docker is not available; Dex-backed OIDC tests may skip." >&2
        return 0
    fi

    step "Starting shared Dex for OIDC tests"
    (
        cd "$REPO_ROOT/docker/utils"
        docker compose up -d --force-recreate dex
    ) || echo "Warning: could not start docker/utils Dex; Dex-backed OIDC tests may skip." >&2
}

npm_install_dir() {
    if [ -f package-lock.json ]; then
        npm ci --no-audit --no-fund
    else
        npm install --no-audit --no-fund --no-package-lock
    fi
}

ensure_playwright_browser() {
    local install_script="$1"

    if [ -n "$install_script" ]; then
        npm run "$install_script"
        return
    fi

    if compgen -G "playwright.config.*" >/dev/null; then
        npx playwright install chromium
    fi
}

run_npm_suite() {
    local rel_dir="$1"
    local label="$2"
    local script_name="$3"
    local playwright_install_script="${4:-}"

    step "$label"
    (
        cd "$REPO_ROOT/$rel_dir"
        npm_install_dir
        ensure_playwright_browser "$playwright_install_script"
        npm run "$script_name"
    )
}

supplementary_server_responding() {
    curl -sf "$KALAMDB_SERVER_URL/health" > /dev/null 2>&1 \
        || curl -sf "$KALAMDB_SERVER_URL/v1/api/healthcheck" > /dev/null 2>&1
}

allocate_free_local_port() {
    python3 - <<'PY'
import socket

with socket.socket() as sock:
    sock.bind(("127.0.0.1", 0))
    print(sock.getsockname()[1])
PY
}

SUPPLEMENTARY_SERVER_PID=""
SUPPLEMENTARY_SERVER_WORK_DIR=""
SUPPLEMENTARY_SERVER_URL=""

cleanup_supplementary_server() {
    if [ -n "$SUPPLEMENTARY_SERVER_PID" ]; then
        kill "$SUPPLEMENTARY_SERVER_PID" 2>/dev/null || true
        wait "$SUPPLEMENTARY_SERVER_PID" 2>/dev/null || true
        SUPPLEMENTARY_SERVER_PID=""
    fi

    if [ -n "$SUPPLEMENTARY_SERVER_WORK_DIR" ]; then
        rm -rf "$SUPPLEMENTARY_SERVER_WORK_DIR"
        SUPPLEMENTARY_SERVER_WORK_DIR=""
    fi
}

trap cleanup_supplementary_server EXIT

supplementary_json_token_from_file() {
    local path="$1"
    node -e 'const fs = require("fs"); const body = JSON.parse(fs.readFileSync(process.argv[1], "utf8")); process.stdout.write(body.access_token || "");' "$path"
}

supplementary_sql_response_has_rows() {
    local path="$1"
    node -e 'const fs = require("fs"); const body = JSON.parse(fs.readFileSync(process.argv[1], "utf8")); const result = Array.isArray(body.results) ? body.results[0] : null; const rows = Array.isArray(result?.rows) ? result.rows.length : 0; const rowCount = typeof result?.row_count === "number" ? result.row_count : rows; process.exit(rowCount > 0 ? 0 : 1);' "$path"
}

supplementary_sql_escape() {
    printf '%s' "$1" | sed "s/'/''/g"
}

supplementary_try_login() {
    local user="$1"
    local password="$2"
    local body_file="$3"
    local status

    status=$(curl -sS -o "$body_file" -w '%{http_code}' \
        -H 'Content-Type: application/json' \
        -d "{\"user\":\"$user\",\"password\":\"$password\"}" \
        "$KALAMDB_SERVER_URL/v1/api/auth/login")
    [[ "$status" == "200" ]]
}

setup_supplementary_auth_if_needed() {
    local auth_tmp_dir
    local status_body
    local login_body
    local root_login_body
    local user_check_body
    local user_sql_body
    local root_token
    local user_sql
    local password_sql
    local check_sql
    local repair_sql
    local repair_status

    auth_tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/kalamdb-supp-auth.XXXXXX")"
    status_body="$auth_tmp_dir/status.json"
    login_body="$auth_tmp_dir/login.json"
    root_login_body="$auth_tmp_dir/root-login.json"
    user_check_body="$auth_tmp_dir/user-check.json"
    user_sql_body="$auth_tmp_dir/user-sql.json"

    if curl -fsS "$KALAMDB_SERVER_URL/v1/api/auth/status" > "$status_body" 2>/dev/null; then
        if grep -Eq '"needs_setup"[[:space:]]*:[[:space:]]*true' "$status_body"; then
            curl -fsS "$KALAMDB_SERVER_URL/v1/api/auth/setup" \
                -H "Content-Type: application/json" \
                -d "{\"user\":\"$KALAMDB_ADMIN_USER\",\"password\":\"$KALAMDB_ADMIN_PASSWORD\",\"root_password\":\"$TEST_ROOT_PASSWORD\",\"email\":null}" \
                >/dev/null
            rm -rf "$auth_tmp_dir"
            return 0
        fi
    fi

    if supplementary_try_login "$KALAMDB_ADMIN_USER" "$KALAMDB_ADMIN_PASSWORD" "$login_body"; then
        rm -rf "$auth_tmp_dir"
        return 0
    fi

    if ! supplementary_try_login root "$TEST_ROOT_PASSWORD" "$root_login_body"; then
        echo "Could not authenticate supplementary admin/root credentials against $KALAMDB_SERVER_URL" >&2
        if [ -s "$login_body" ]; then
            echo "Admin login response:" >&2
            cat "$login_body" >&2
        fi
        if [ -s "$root_login_body" ]; then
            echo "Root login response:" >&2
            cat "$root_login_body" >&2
        fi
        rm -rf "$auth_tmp_dir"
        return 1
    fi

    root_token="$(supplementary_json_token_from_file "$root_login_body")"
    if [ -z "$root_token" ]; then
        echo "Supplementary root login returned no access token for $KALAMDB_SERVER_URL" >&2
        cat "$root_login_body" >&2
        rm -rf "$auth_tmp_dir"
        return 1
    fi

    user_sql="$(supplementary_sql_escape "$KALAMDB_ADMIN_USER")"
    password_sql="$(supplementary_sql_escape "$KALAMDB_ADMIN_PASSWORD")"
    check_sql="SELECT user_id FROM system.users WHERE user_id = '$user_sql' LIMIT 1"

    curl -sS -o "$user_check_body" \
        -H 'Content-Type: application/json' \
        -H "Authorization: Bearer $root_token" \
        -d "{\"sql\":\"$check_sql\"}" \
        "$KALAMDB_SERVER_URL/v1/api/sql" >/dev/null

    if supplementary_sql_response_has_rows "$user_check_body"; then
        repair_sql="ALTER USER '$user_sql' SET PASSWORD '$password_sql'; ALTER USER '$user_sql' SET ROLE 'dba';"
    else
        repair_sql="CREATE USER '$user_sql' WITH PASSWORD '$password_sql' ROLE 'dba'"
    fi

    repair_status=$(curl -sS -o "$user_sql_body" -w '%{http_code}' \
        -H 'Content-Type: application/json' \
        -H "Authorization: Bearer $root_token" \
        -d "{\"sql\":\"$repair_sql\"}" \
        "$KALAMDB_SERVER_URL/v1/api/sql")
    if [[ "$repair_status" != "200" ]] && ! grep -Eiq 'already exists|duplicate|conflict|idempotent' "$user_sql_body"; then
        echo "Failed to ensure supplementary admin user '$KALAMDB_ADMIN_USER'." >&2
        cat "$user_sql_body" >&2
        rm -rf "$auth_tmp_dir"
        return 1
    fi

    if ! supplementary_try_login "$KALAMDB_ADMIN_USER" "$KALAMDB_ADMIN_PASSWORD" "$login_body"; then
        echo "Supplementary admin credentials still cannot log in after repair." >&2
        cat "$login_body" >&2
        rm -rf "$auth_tmp_dir"
        return 1
    fi

    rm -rf "$auth_tmp_dir"
}

start_supplementary_server_if_needed() {
    local server_log
    local server_pid_file
    local server_port

    if [ "$SERVER_TYPE" != "fresh" ]; then
        return 0
    fi

    if [ -n "$SUPPLEMENTARY_SERVER_PID" ]; then
        return 0
    fi

    server_port="$(allocate_free_local_port)"
    SUPPLEMENTARY_SERVER_URL="http://127.0.0.1:$server_port"
    export KALAMDB_SERVER_URL="$SUPPLEMENTARY_SERVER_URL"
    export KALAMDB_URL="$SUPPLEMENTARY_SERVER_URL"
    export KALAMDB_TEST_URL="$SUPPLEMENTARY_SERVER_URL"
    export KALAM_URL="$SUPPLEMENTARY_SERVER_URL"

    SUPPLEMENTARY_SERVER_WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/kalamdb-supplementary-server.XXXXXX")"
    server_log="$SUPPLEMENTARY_SERVER_WORK_DIR/server.log"
    server_pid_file="$SUPPLEMENTARY_SERVER_WORK_DIR/server.pid"

    KALAMDB_SERVER_WORK_DIR="$SUPPLEMENTARY_SERVER_WORK_DIR" \
        KALAMDB_SERVER_LOG="$server_log" \
        KALAMDB_SERVER_PID_FILE="$server_pid_file" \
        KALAMDB_SERVER_WAIT_SECONDS="${KALAMDB_SERVER_WAIT_SECONDS:-180}" \
        KALAMDB_URL="$SUPPLEMENTARY_SERVER_URL" \
        bash "$REPO_ROOT/scripts/start-sdk-test-server.sh"

    SUPPLEMENTARY_SERVER_PID="$(cat "$server_pid_file")"
    setup_supplementary_auth_if_needed
}

run_supplementary_suites() {
    step "Checking supplementary suite toolchain"
    require_cmd node
    require_cmd npm
    require_cmd flutter

    start_supplementary_server_if_needed

    export KALAMDB_USER="$KALAMDB_ADMIN_USER"
    export KALAMDB_PASSWORD="$KALAMDB_ADMIN_PASSWORD"
    export KALAMDB_TEST_USER="$KALAMDB_ADMIN_USER"
    export KALAMDB_TEST_PASSWORD="$KALAMDB_ADMIN_PASSWORD"

    run_npm_suite "link/sdks/typescript/client" "Running TypeScript client SDK tests" "test"
    run_npm_suite \
        "link/sdks/typescript/client" \
        "Running TypeScript client SDK Playwright tests" \
        "test:browser" \
        "test:browser:install"
    run_npm_suite "link/sdks/typescript/cli" "Running TypeScript CLI package tests" "test"
    run_npm_suite "link/sdks/typescript/consumer" "Running TypeScript consumer SDK tests" "test"
    run_npm_suite "link/sdks/typescript/orm" "Running TypeScript ORM SDK tests" "test"
    run_npm_suite "link/sdks/typescript/react" "Running TypeScript React SDK unit tests" "test"
    run_npm_suite \
        "link/sdks/typescript/react" \
        "Running TypeScript React SDK Playwright tests" \
        "test:e2e" \
        "test:e2e:install"

    run_npm_suite "examples/chat-with-ai" "Running chat-with-ai example tests" "test"
    run_npm_suite "examples/react-ai-chat" "Running react-ai-chat example tests" "test"
    run_npm_suite "examples/simple-typescript" "Running simple-typescript Playwright tests" "test"
    run_npm_suite "examples/summarizer-agent" "Running summarizer-agent example tests" "test"
    run_npm_suite "ui" "Running admin UI tests" "test:ci"
    run_npm_suite \
        "ui" \
        "Running admin UI Playwright tests" \
        "test:e2e" \
        "test:e2e:install"

    step "Running Dart SDK tests"
    (
        cd "$REPO_ROOT/link/sdks/dart"
        ./test.sh
    )
}

build_test_cmd() {
    local test_filter="$1"
    TEST_CMD=(
        cargo nextest run
    )

    local filter_targets_smoke=false
    if [ -n "$test_filter" ] && [[ "$test_filter" == smoke* ]]; then
        filter_targets_smoke=true
    fi

    if [ -z "$TEST_TARGET" ] && [ "$filter_targets_smoke" = false ]; then
        TEST_CMD+=(--all-targets)
    fi

    local e2e_features=()

    if [ ${#PACKAGE_FILTERS[@]} -gt 0 ]; then
        local package
        for package in "${PACKAGE_FILTERS[@]}"; do
            TEST_CMD+=(-p "$package")

            case "$package" in
                kalam-cli|kalamdb-server)
                    e2e_features+=("e2e-tests")
                    ;;
            esac
        done

        if [ ${#e2e_features[@]} -gt 0 ]; then
            TEST_CMD+=(--features "${e2e_features[*]}")
        fi
    else
        TEST_CMD+=(--workspace)
        # The PostgreSQL extension crate is tested via the dedicated pgrx workflow,
        # not through generic cargo test/nextest targets.
        TEST_CMD+=(--exclude "kalam-pg-extension")
        TEST_CMD+=(--features "kalam-cli/e2e-tests kalamdb-server/e2e-tests")
    fi

    if [ -n "$TEST_TARGET" ]; then
        TEST_CMD+=(--test "$TEST_TARGET")
    fi

    # nextest.toml already serializes the stateful kalam-cli / kalam-link
    # packages. Do not force a global `-j 1` here, otherwise the entire
    # workspace becomes single-file even when only those packages need it.
    if [ -n "$TEST_JOBS" ]; then
        TEST_CMD+=(-j "$TEST_JOBS")
    fi

    if [ -n "$test_filter" ]; then
        if [ -z "$TEST_TARGET" ] && [ "$filter_targets_smoke" = true ]; then
            TEST_CMD+=(--test smoke)
            if [[ "$test_filter" != "smoke" ]]; then
                TEST_CMD+=("$test_filter")
            fi
        else
            TEST_CMD+=("$test_filter")
        fi
    fi

    if [ -n "$NOCAPTURE" ]; then
        TEST_CMD+=(--no-capture)
    fi
}

run_single_test() {
    local test_filter="$1"
    build_test_cmd "$test_filter"
    echo "Executing: ${TEST_CMD[*]}"
    echo ""
    if [ "$SERVER_TYPE" = "fresh" ]; then
        env -u KALAMDB_SERVER_URL -u KALAMDB_CLUSTER_URLS "${TEST_CMD[@]}"
    else
        "${TEST_CMD[@]}"
    fi
}

run_test_list() {
    local test_file="$1"
    local input_path="$test_file"
    local test_filter=""
    local total=0
    local passed=0
    local exit_code=0

    if [ "$test_file" = "-" ]; then
        input_path="/dev/stdin"
    fi

    while IFS= read -r test_filter || [ -n "$test_filter" ]; do
        test_filter="${test_filter%$'\r'}"
        case "$test_filter" in
            ''|\#*)
                continue
                ;;
        esac

        total=$((total + 1))
        echo ""
        echo "=== RUN $test_filter ==="
        if run_single_test "$test_filter"; then
            passed=$((passed + 1))
        else
            exit_code=$?
            echo ""
            echo "Failed test: $test_filter"
            echo "Summary: $passed passed before first failure ($total attempted)"
            return $exit_code
        fi
    done < "$input_path"

    echo ""
    echo "Summary: $passed/$total tests passed from rerun list"
}

single_package_name() {
    if [ ${#PACKAGE_FILTERS[@]} -eq 1 ]; then
        echo "${PACKAGE_FILTERS[0]}"
    fi
}

# Run tests from workspace root
cd "$REPO_ROOT"

prebuild_cli_oidc_server_binary_if_needed
ensure_dex_for_oidc_tests

if [ -n "$TEST_LIST_FILE" ]; then
    run_test_list "$TEST_LIST_FILE"
else
    run_single_test "$TEST_FILTER"
fi

if [ "$RUN_SUPPLEMENTARY_SUITES" = true ]; then
    run_supplementary_suites
fi
