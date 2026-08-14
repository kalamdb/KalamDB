#!/usr/bin/env bash
# Download comparison server binaries into benchv2/comparison/bin/
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$ROOT/bin"
mkdir -p "$BIN"

ARCH="$(uname -m)"
OS="$(uname -s)"

kalam_asset=""
trail_asset=""
pb_asset=""

case "$OS-$ARCH" in
  Darwin-arm64)
    kalam_asset="kalamdb-server-0.5.5-rc.1-macos-aarch64.tar.gz"
    trail_asset="trailbase_v0.32.1_arm64_apple_darwin.zip"
    pb_asset="pocketbase_0.29.3_darwin_arm64.zip"
    ;;
  Darwin-x86_64)
    echo "No KalamDB macos-x86_64 release asset in v0.5.5-rc.1; use arm64 or Linux." >&2
    exit 1
    ;;
  Linux-aarch64|Linux-arm64)
    kalam_asset="kalamdb-server-0.5.5-rc.1-linux-aarch64.tar.gz"
    trail_asset="trailbase_v0.32.1_arm64_linux.zip"
    pb_asset="pocketbase_0.29.3_linux_arm64.zip"
    ;;
  Linux-x86_64)
    kalam_asset="kalamdb-server-0.5.5-rc.1-linux-x86_64.tar.gz"
    trail_asset="trailbase_v0.32.1_x86_64_linux.zip"
    pb_asset="pocketbase_0.29.3_linux_amd64.zip"
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

# KalamDB release
if [[ ! -x "$BIN/kalamdb-server" ]]; then
  download "https://github.com/kalamdb/KalamDB/releases/download/v0.5.5-rc.1/${kalam_asset}" "$BIN/${kalam_asset}"
  tar -xzf "$BIN/${kalam_asset}" -C "$BIN"
  # tarball may contain a bare binary named with the version suffix
  if [[ -f "$BIN/kalamdb-server" ]]; then
    chmod +x "$BIN/kalamdb-server"
  else
    found="$(find "$BIN" -maxdepth 1 -type f -name 'kalamdb-server*' ! -name '*.tar.gz' | head -1)"
    cp "$found" "$BIN/kalamdb-server"
    chmod +x "$BIN/kalamdb-server"
  fi
fi

# TrailBase release
if [[ ! -x "$BIN/trail" ]]; then
  download "https://github.com/trailbaseio/trailbase/releases/download/v0.32.1/${trail_asset}" "$BIN/${trail_asset}"
  unzip -o "$BIN/${trail_asset}" -d "$BIN/trailbase-extract"
  cp "$BIN/trailbase-extract/trail" "$BIN/trail"
  chmod +x "$BIN/trail"
fi

# PocketBase release
if [[ ! -x "$BIN/pocketbase" ]]; then
  download "https://github.com/pocketbase/pocketbase/releases/download/v0.29.3/${pb_asset}" "$BIN/${pb_asset}"
  unzip -o "$BIN/${pb_asset}" -d "$BIN/pocketbase-extract"
  cp "$BIN/pocketbase-extract/pocketbase" "$BIN/pocketbase"
  chmod +x "$BIN/pocketbase"
fi

echo "Binaries ready in $BIN"
ls -la "$BIN/kalamdb-server" "$BIN/trail" "$BIN/pocketbase"
