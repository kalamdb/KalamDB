#!/usr/bin/env bash
# Download comparison server binaries into benchv2/comparison/bin/
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$ROOT/bin"
mkdir -p "$BIN"

# GitHub release pins. Bump these and re-run this script to refresh ./bin.
TRAIL_VERSION="v0.34.4"
PB_VERSION="0.40.5"
SURREAL_VERSION="v3.3.0"
SPACETIME_VERSION="v2.16.1"
CONVEX_VERSION="precompiled-2026-11-19-9fde7cb"
KALAM_RELEASE="v0.5.5-rc.1"

ARCH="$(uname -m)"
OS="$(uname -s)"

kalam_asset=""
trail_asset=""
pb_asset=""
surreal_asset=""

case "$OS-$ARCH" in
  Darwin-arm64)
    kalam_asset="kalamdb-server-0.5.5-rc.1-macos-aarch64.tar.gz"
    trail_asset="trailbase_${TRAIL_VERSION}_aarch64_apple_darwin.zip"
    pb_asset="pocketbase_${PB_VERSION}_darwin_arm64.zip"
    surreal_asset="surreal-${SURREAL_VERSION}.darwin-arm64.tgz"
    spacetime_asset="spacetime-aarch64-apple-darwin.tar.gz"
    convex_asset="convex-local-backend-aarch64-apple-darwin.zip"
    ;;
  Darwin-x86_64)
    echo "No KalamDB macos-x86_64 release asset in ${KALAM_RELEASE}; use arm64 or Linux." >&2
    exit 1
    ;;
  Linux-aarch64|Linux-arm64)
    kalam_asset="kalamdb-server-0.5.5-rc.1-linux-aarch64.tar.gz"
    trail_asset="trailbase_${TRAIL_VERSION}_aarch64_linux.zip"
    pb_asset="pocketbase_${PB_VERSION}_linux_arm64.zip"
    surreal_asset="surreal-${SURREAL_VERSION}.linux-arm64.tgz"
    spacetime_asset="spacetime-aarch64-unknown-linux-gnu.tar.gz"
    convex_asset="convex-local-backend-aarch64-unknown-linux-gnu.zip"
    ;;
  Linux-x86_64)
    kalam_asset="kalamdb-server-0.5.5-rc.1-linux-x86_64.tar.gz"
    trail_asset="trailbase_${TRAIL_VERSION}_x86_64_linux.zip"
    pb_asset="pocketbase_${PB_VERSION}_linux_amd64.zip"
    surreal_asset="surreal-${SURREAL_VERSION}.linux-amd64.tgz"
    spacetime_asset="spacetime-x86_64-unknown-linux-gnu.tar.gz"
    convex_asset="convex-local-backend-x86_64-unknown-linux-gnu.zip"
    ;;
  *)
    echo "Unsupported platform: $OS-$ARCH" >&2
    exit 1
    ;;
esac

download() {
  local url="$1"
  local out="$2"
  if [[ -f "$out" ]]; then
    echo "exists: $out"
    return
  fi
  echo "Downloading $url"
  curl -fL --retry 3 -o "$out" "$url"
}

needs_refresh() {
  local dest="$1"
  local stamp="$2"
  local expected="$3"
  if [[ -x "$dest" && -f "$stamp" && "$(cat "$stamp")" == "$expected" ]]; then
    echo "up to date: $dest ($expected)"
    return 1
  fi
  return 0
}

# KalamDB release (optional local fallback; bake-off prefers target/release/kalamdb-server)
if [[ ! -x "$BIN/kalamdb-server" ]]; then
  download "https://github.com/kalamdb/KalamDB/releases/download/${KALAM_RELEASE}/${kalam_asset}" "$BIN/${kalam_asset}"
  tar -xzf "$BIN/${kalam_asset}" -C "$BIN"
  if [[ -f "$BIN/kalamdb-server" ]]; then
    chmod +x "$BIN/kalamdb-server"
  else
    found="$(find "$BIN" -maxdepth 1 -type f -name 'kalamdb-server*' ! -name '*.tar.gz' | head -1)"
    cp "$found" "$BIN/kalamdb-server"
    chmod +x "$BIN/kalamdb-server"
  fi
fi

# TrailBase release
if needs_refresh "$BIN/trail" "$BIN/.trail-version" "$TRAIL_VERSION"; then
  download "https://github.com/trailbaseio/trailbase/releases/download/${TRAIL_VERSION}/${trail_asset}" "$BIN/${trail_asset}"
  rm -rf "$BIN/trailbase-extract"
  unzip -o "$BIN/${trail_asset}" -d "$BIN/trailbase-extract"
  cp "$BIN/trailbase-extract/trail" "$BIN/trail"
  chmod +x "$BIN/trail"
  printf '%s\n' "$TRAIL_VERSION" >"$BIN/.trail-version"
fi

# PocketBase release
if needs_refresh "$BIN/pocketbase" "$BIN/.pocketbase-version" "$PB_VERSION"; then
  download "https://github.com/pocketbase/pocketbase/releases/download/v${PB_VERSION}/${pb_asset}" "$BIN/${pb_asset}"
  rm -rf "$BIN/pocketbase-extract"
  unzip -o "$BIN/${pb_asset}" -d "$BIN/pocketbase-extract"
  cp "$BIN/pocketbase-extract/pocketbase" "$BIN/pocketbase"
  chmod +x "$BIN/pocketbase"
  printf '%s\n' "$PB_VERSION" >"$BIN/.pocketbase-version"
fi

# SurrealDB release (RocksDB-backed official binary)
if needs_refresh "$BIN/surreal" "$BIN/.surreal-version" "$SURREAL_VERSION"; then
  download "https://github.com/surrealdb/surrealdb/releases/download/${SURREAL_VERSION}/${surreal_asset}" "$BIN/${surreal_asset}"
  mkdir -p "$BIN/surreal-extract"
  tar -xzf "$BIN/${surreal_asset}" -C "$BIN/surreal-extract"
  found="$(find "$BIN/surreal-extract" -type f -name 'surreal' | head -1)"
  if [[ -z "$found" ]]; then
    found="$(find "$BIN/surreal-extract" -type f ! -name '*.tgz' | head -1)"
  fi
  cp "$found" "$BIN/surreal"
  chmod +x "$BIN/surreal"
  printf '%s\n' "$SURREAL_VERSION" >"$BIN/.surreal-version"
fi

# SpaceTimeDB CLI + standalone (file-backed; module is built at publish time)
if needs_refresh "$BIN/spacetimedb-cli" "$BIN/.spacetimedb-version" "$SPACETIME_VERSION"; then
  download "https://github.com/clockworklabs/SpacetimeDB/releases/download/${SPACETIME_VERSION}/${spacetime_asset}" "$BIN/${spacetime_asset}"
  rm -rf "$BIN/spacetime-extract"
  mkdir -p "$BIN/spacetime-extract"
  tar -xzf "$BIN/${spacetime_asset}" -C "$BIN/spacetime-extract"
  cp "$BIN/spacetime-extract/spacetimedb-cli" "$BIN/spacetimedb-cli"
  cp "$BIN/spacetime-extract/spacetimedb-standalone" "$BIN/spacetimedb-standalone"
  chmod +x "$BIN/spacetimedb-cli" "$BIN/spacetimedb-standalone"
  ln -sfn spacetimedb-cli "$BIN/spacetime"
  printf '%s\n' "$SPACETIME_VERSION" >"$BIN/.spacetimedb-version"
fi

if needs_refresh "$BIN/convex-local-backend" "$BIN/.convex-version" "$CONVEX_VERSION"; then
  download "https://github.com/get-convex/convex-backend/releases/download/${CONVEX_VERSION}/${convex_asset}" "$BIN/${convex_asset}"
  rm -rf "$BIN/convex-extract"
  mkdir -p "$BIN/convex-extract"
  unzip -o "$BIN/${convex_asset}" -d "$BIN/convex-extract"
  cp "$BIN/convex-extract/convex-local-backend" "$BIN/convex-local-backend"
  chmod +x "$BIN/convex-local-backend"
  printf '%s\n' "$CONVEX_VERSION" >"$BIN/.convex-version"
fi

echo "Binaries ready in $BIN"
echo "  TrailBase ${TRAIL_VERSION}"
echo "  PocketBase v${PB_VERSION}"
echo "  SurrealDB ${SURREAL_VERSION}"
echo "  SpaceTimeDB ${SPACETIME_VERSION}"
echo "  Convex ${CONVEX_VERSION}"
ls -la "$BIN/kalamdb-server" "$BIN/trail" "$BIN/pocketbase" "$BIN/surreal" "$BIN/spacetimedb-cli" "$BIN/spacetimedb-standalone" "$BIN/convex-local-backend"
