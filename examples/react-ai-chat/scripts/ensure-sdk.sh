#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"

ensure_package() {
  local dir="$1"
  local build_cmd="$2"
  if [ ! -d "$ROOT_DIR/$dir/dist" ]; then
    (cd "$ROOT_DIR/$dir" && npm install --no-package-lock && npm run "$build_cmd")
  fi
}

ensure_package "link/sdks/typescript/client" "build:ts"
ensure_package "link/sdks/typescript/orm" "build"
ensure_package "link/sdks/typescript/react-old" "build"