#!/usr/bin/env bash
# Run every top-level offline test, including native package validation.
set -euo pipefail
cd "$(dirname "$0")"

tests=()
for test_file in test/*_test.dart; do
  case "$test_file" in
    test/live_server_test.dart) continue ;;
  esac
  tests+=("$test_file")
done

# Keep live-server suites opt-in and independent of the caller's environment.
export KALAM_INTEGRATION_TEST=0
if [[ -n "${DART_SDK_MODELS_MACHINE_OUTPUT:-}" ]]; then
  mkdir -p "$(dirname "$DART_SDK_MODELS_MACHINE_OUTPUT")"
  flutter test --machine "$@" "${tests[@]}" | tee "$DART_SDK_MODELS_MACHINE_OUTPUT"
else
  flutter test "$@" "${tests[@]}"
fi
