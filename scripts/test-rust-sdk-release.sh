#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
SDK_DIR="$ROOT_DIR/link/sdks/rust"
SERVER_URL="${KALAMDB_URL:-http://localhost:2900}"
SERVER_LOG="${RUST_SDK_SERVER_LOG:-$ROOT_DIR/rust-sdk-server.log}"
TEST_OUTPUT="${RUST_SDK_TEST_OUTPUT:-$ROOT_DIR/rust-sdk-test-output.txt}"
SERVER_BIN="${KALAMDB_SERVER_BIN:-}"
SKIP_SERVER_START="${KALAMDB_SKIP_SERVER_START:-false}"
SERVER_PID=""

cleanup() {
  if [[ -n "${SERVER_PID:-}" ]]; then
    kill "$SERVER_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

{
  echo "Running Rust SDK tests"
  echo "Server URL: $SERVER_URL"
} >"$TEST_OUTPUT"

if [[ "$SKIP_SERVER_START" != "true" && -n "$SERVER_BIN" ]]; then
  SERVER_WORK_DIR="$ROOT_DIR/rust-sdk-test-server"
  mkdir -p "$SERVER_WORK_DIR"
  KALAMDB_SERVER_BIN="$SERVER_BIN" \
    KALAMDB_SERVER_WORK_DIR="$SERVER_WORK_DIR" \
    KALAMDB_SERVER_LOG="$SERVER_LOG" \
    KALAMDB_SERVER_PID_FILE="$SERVER_WORK_DIR/server.pid" \
    KALAMDB_SERVER_WAIT_SECONDS="${KALAMDB_SERVER_WAIT_SECONDS:-120}" \
    KALAMDB_URL="$SERVER_URL" \
    bash "$SCRIPT_DIR/start-sdk-test-server.sh"
  SERVER_PID="$(cat "$SERVER_WORK_DIR/server.pid")"
  export KALAMDB_SERVER_URL="$SERVER_URL"
  export KALAMDB_ROOT_PASSWORD="${KALAMDB_ROOT_PASSWORD:-kalamdb123}"
fi

set +e
(
  cd "$SDK_DIR"
  bash ./test.sh
) 2>&1 | tee -a "$TEST_OUTPUT"
TEST_STATUS=${PIPESTATUS[0]}
set -e

python3 - <<'PY' "$TEST_OUTPUT" "$TEST_STATUS"
import re
import sys
from pathlib import Path

output_path = Path(sys.argv[1])
status = int(sys.argv[2])
text = output_path.read_text(encoding="utf-8", errors="replace")

passed = 0
failed = 0
for match in re.finditer(r"^test result: ok\. (\d+) passed", text, flags=re.MULTILINE):
    passed += int(match.group(1))
for match in re.finditer(r"^test result: FAILED\. (\d+) passed; (\d+) failed", text, flags=re.MULTILINE):
    passed += int(match.group(1))
    failed += int(match.group(2))

total = passed + failed
if total == 0 and status == 0:
    passed = 1
    total = 1
elif total == 0 and status != 0:
    failed = 1
    total = 1

with output_path.open("a", encoding="utf-8") as handle:
    handle.write(f"# tests {total}\n")
    handle.write(f"# pass {passed}\n")
    handle.write(f"# fail {failed}\n")
PY

exit "$TEST_STATUS"
