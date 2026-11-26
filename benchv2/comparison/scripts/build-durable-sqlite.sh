#!/usr/bin/env bash
# Package the pinned SQLite-backed servers with synchronous commit settings.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$ROOT/bin"
SYSTEM="${1:-all}"

if [[ $# -gt 1 ]]; then
  echo "Usage: $0 [all|trailbase|pocketbase|convex]" >&2
  exit 2
fi
case "$SYSTEM" in
  all|trailbase|pocketbase|convex) ;;
  -h|--help) echo "Usage: $0 [all|trailbase|pocketbase|convex]"; exit 0 ;;
  *) echo "Unknown system '$SYSTEM'. Use all, trailbase, pocketbase, or convex." >&2; exit 2 ;;
esac

mkdir -p "$BIN"
WORK="$(mktemp -d "$BIN/.build-durable-sqlite.XXXXXXXX")"
trap 'rm -rf -- "$WORK"' EXIT

checkout() {
  local repository="$1" ref="$2" commit="$3" source="$4"
  git clone --quiet --depth 1 --branch "$ref" --recurse-submodules --shallow-submodules \
    "https://github.com/$repository.git" "$source"
  if [[ "$(git -C "$source" rev-parse HEAD)" != "$commit" ]]; then
    echo "Unexpected commit for $repository $ref (expected $commit)" >&2
    exit 1
  fi
}

replace_once() {
  python3 - "$@" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
before, after = (argument.encode() for argument in sys.argv[2:])
contents = path.read_bytes()
if contents.count(before) != 1:
    raise SystemExit(f"Expected exactly one durability patch site in {path}")
path.write_bytes(contents.replace(before, after, 1))
PY
}

publish() {
  local artifact="$1" name="$2" ref="$3" commit="$4" sqlite="$5"
  local stamp=".${name}-version"
  install -m 755 "$artifact" "$WORK/$name"
  printf '%s commit=%s sqlite=%s\n' "$ref" "$commit" "$sqlite" > "$WORK/$stamp"
  mv -f "$WORK/$name" "$BIN/$name"
  mv -f "$WORK/$stamp" "$BIN/$stamp"
  echo "Built $BIN/$name ($ref, SQLite $sqlite)"
}

build_trailbase() {
  local ref="v0.34.4" commit="12d3343d7d78c8e4dd65af264a3244f472de450d"
  local source="$WORK/trailbase" target="$WORK/trailbase-target" host
  checkout trailbaseio/trailbase "$ref" "$commit" "$source"
  replace_once "$source/crates/extension/src/lib.rs" \
    $'// Sync the file system less often.\n  conn.pragma_update(None, "synchronous", "NORMAL")?;' \
    'conn.pragma_update(None, "synchronous", "FULL")?;'
  (
    cd "$source"
    host="$(rustc +1.98.1 -vV | awk '/^host:/ { print $2 }')"
    CI=true pnpm install --frozen-lockfile
    PNPM_OFFLINE=TRUE CARGO_PROFILE_RELEASE_LTO=fat CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 \
      cargo +1.98.1 build --locked --release --target "$host" --target-dir "$target" \
        --features=geos-static --bin=trail
    publish "$target/$host/release/trail" trail-durable "$ref" "$commit" wal/full
  )
}

build_pocketbase() {
  local ref="v0.40.5" commit="6f370ea741f33adf889eef02e3c1b884a47aa522"
  local source="$WORK/pocketbase" artifact="$WORK/pocketbase-build"
  checkout pocketbase/pocketbase "$ref" "$commit" "$source"
  replace_once "$source/core/db_connect.go" \
    '_pragma=synchronous(NORMAL)' '_pragma=synchronous(FULL)'
  (
    cd "$source"
    CGO_ENABLED=0 GOTOOLCHAIN=go1.27.2 go build -mod=readonly -trimpath \
      -ldflags "-s -w -X github.com/pocketbase/pocketbase.Version=${ref#v}" \
      -o "$artifact" ./examples/base
  )
  publish "$artifact" pocketbase-durable "$ref" "$commit" wal/full
}

build_convex() {
  local ref="precompiled-2026-11-19-9fde7cb" commit="9fde7cb10083b101c27cd99a83553ce59d7b2034"
  local source="$WORK/convex" target="$WORK/convex-target" host
  checkout convex/convex-backend "$ref" "$commit" "$source"
  replace_once "$source/crates/sqlite/src/lib.rs" \
    'let connection = Connection::open(path)?;' \
    $'let connection = Connection::open(path)?;\n        connection.pragma_update(None, "journal_mode", "DELETE")?;\n        connection.pragma_update(None, "synchronous", "EXTRA")?;'
  (
    cd "$source"
    host="$(rustc -vV | awk '/^host:/ { print $2 }')"
    npm clean-install --prefix scripts
    just install-js
    # The build scripts must bundle JS for this checkout.
    unset CONVEX_PREBUILT_JS
    cargo build --locked --release --target "$host" --target-dir "$target" \
      -p local_backend --bin convex-local-backend
    publish "$target/$host/release/convex-local-backend" \
      convex-local-backend-durable "$ref" "$commit" delete/extra
  )
}

case "$SYSTEM" in
  all)
    build_trailbase
    build_pocketbase
    build_convex
    ;;
  trailbase) build_trailbase ;;
  pocketbase) build_pocketbase ;;
  convex) build_convex ;;
esac
