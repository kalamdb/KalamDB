#!/usr/bin/env bash
# Python SDK gates that need more than `pytest tests/test_schema.py`:
# generated bindings that are up to date, strictly typed, and exercised
# against a freshly started server.
#
# A server is started for the smoke test unless KALAMDB_SKIP_SERVER_START=true,
# in which case one is expected to be running at KALAMDB_URL already.
# KALAMDB_SERVER_BIN names a prebuilt binary; without it the server is built
# from source.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
SDK_DIR="$ROOT_DIR/link/sdks/python"
EXAMPLE_DIR="$ROOT_DIR/examples/simple-python"
SERVER_URL="${KALAMDB_URL:-http://localhost:2900}"
SERVER_LOG="${PYTHON_SDK_SERVER_LOG:-$ROOT_DIR/python-sdk-server.log}"
SERVER_BIN="${KALAMDB_SERVER_BIN:-}"
SKIP_SERVER_START="${KALAMDB_SKIP_SERVER_START:-false}"
KALAM_BIN="${KALAM_BIN:-$ROOT_DIR/target/debug/kalam}"
PYTHON_BIN="${PYTHON_BIN:-python3}"
SERVER_PID=""

cleanup() {
  if [[ -n "${SERVER_PID:-}" ]]; then
    # The harness backgrounds the server inside a subshell, so the recorded pid
    # is the subshell and the server is its child. Killing only the parent
    # leaves a server holding the port behind.
    pkill -P "$SERVER_PID" 2>/dev/null || true
    kill "$SERVER_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

if [[ ! -x "$KALAM_BIN" ]]; then
  echo "kalam CLI not found at $KALAM_BIN; build it with 'cargo build -p kalam-cli --bin kalam'" >&2
  exit 1
fi

echo "==> generated bindings are strictly typed and round trip"
"$PYTHON_BIN" "$ROOT_DIR/link/tests/sdk-contracts/check_generated.py" \
  --python --python-bin "$PYTHON_BIN" --kalam "$KALAM_BIN" \
  --skip-typescript --skip-dart

echo "==> checked-in example matches the current contract"
(cd "$EXAMPLE_DIR" && "$KALAM_BIN" schema gen --check --languages python --agent --no-spinner)

echo "==> example and SDK pass mypy --strict"
(cd "$EXAMPLE_DIR" && "$PYTHON_BIN" -m mypy --strict --explicit-package-bases generated/kalam.py main.py)
(cd "$SDK_DIR" && "$PYTHON_BIN" -m mypy --strict kalamdb/schema)

echo "==> unit tests (no server)"
(cd "$SDK_DIR" && "$PYTHON_BIN" -m pytest tests/test_schema.py tests/test_codecs.py -q)

if [[ "$SKIP_SERVER_START" == "true" ]]; then
  echo "==> using the server already running at $SERVER_URL"
else
  if [[ -n "$SERVER_BIN" && ! -f "$SERVER_BIN" ]]; then
    echo "KALAMDB_SERVER_BIN is set to $SERVER_BIN, which does not exist" >&2
    exit 1
  fi
  SERVER_WORK_DIR="$ROOT_DIR/python-sdk-test-server"
  mkdir -p "$SERVER_WORK_DIR"
  # An empty KALAMDB_SERVER_BIN is not a reason to skip the smoke test: the
  # harness builds the server from source when it is not given one.
  KALAMDB_SERVER_BIN="$SERVER_BIN" \
    KALAMDB_SERVER_WORK_DIR="$SERVER_WORK_DIR" \
    KALAMDB_SERVER_LOG="$SERVER_LOG" \
    KALAMDB_SERVER_PID_FILE="$SERVER_WORK_DIR/server.pid" \
    KALAMDB_SERVER_WAIT_SECONDS="${KALAMDB_SERVER_WAIT_SECONDS:-120}" \
    KALAMDB_URL="$SERVER_URL" \
    bash "$SCRIPT_DIR/start-sdk-test-server.sh"
  SERVER_PID="$(cat "$SERVER_WORK_DIR/server.pid")"
fi

echo "==> smoke test against a fresh server"
(
  cd "$SDK_DIR"
  KALAMDB_TEST_URL="$SERVER_URL" \
    KALAMDB_TEST_USER="${KALAMDB_USER:-root}" \
    KALAMDB_TEST_PASSWORD="${KALAMDB_ROOT_PASSWORD:-kalamdb123}" \
    "$PYTHON_BIN" -m pytest tests/test_schema_live.py -q
)

echo "Python SDK release checks passed."
