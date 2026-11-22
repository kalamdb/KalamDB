#!/usr/bin/env bash
# Download comparison server binaries into benchv2/comparison/bin/
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$ROOT/bin"
mkdir -p "$BIN"

# The other benchmark servers are built locally for the verified workload.
SURREAL_VERSION="v3.3.0"
SPACETIME_VERSION="v2.16.1"

if [[ $# == 0 ]]; then
  set -- surrealdb spacetimedb
fi
FETCH_SURREAL=0
FETCH_SPACETIME=0
for system in "$@"; do
  case "$system" in
    surrealdb) FETCH_SURREAL=1 ;;
    spacetimedb) FETCH_SPACETIME=1 ;;
    *) echo "Unknown release binary: $system" >&2; exit 2 ;;
  esac
done

ARCH="$(uname -m)"
OS="$(uname -s)"

case "$OS-$ARCH" in
  Darwin-arm64)
    surreal_asset="surreal-${SURREAL_VERSION}.darwin-arm64.tgz"
    spacetime_asset="spacetime-aarch64-apple-darwin.tar.gz"
    ;;
  Linux-aarch64|Linux-arm64)
    surreal_asset="surreal-${SURREAL_VERSION}.linux-arm64.tgz"
    spacetime_asset="spacetime-aarch64-unknown-linux-gnu.tar.gz"
    ;;
  Linux-x86_64)
    surreal_asset="surreal-${SURREAL_VERSION}.linux-amd64.tgz"
    spacetime_asset="spacetime-x86_64-unknown-linux-gnu.tar.gz"
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

# SurrealDB release (RocksDB-backed official binary)
if [[ "$FETCH_SURREAL" == 1 ]] && needs_refresh "$BIN/surreal" "$BIN/.surreal-version" "$SURREAL_VERSION"; then
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
if [[ "$FETCH_SPACETIME" == 1 ]] && needs_refresh "$BIN/spacetimedb-cli" "$BIN/.spacetimedb-version" "$SPACETIME_VERSION"; then
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

echo "Binaries ready in $BIN"
