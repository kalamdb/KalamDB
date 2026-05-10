#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
    echo "Usage: $0 <asset-name> <asset-api-url> <target-binary-path>" >&2
    exit 1
fi

ASSET_NAME="$1"
ASSET_API_URL="$2"
TARGET_BINARY_PATH="$3"
GITHUB_TOKEN="${GITHUB_TOKEN:-}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORK_DIR="${SCRIPT_DIR}/../target/sdk-release-server"
ARCHIVE_PATH="$WORK_DIR/$ASSET_NAME"
EXTRACT_DIR="$WORK_DIR/extracted"

mkdir -p "$WORK_DIR" "$EXTRACT_DIR" "$(dirname "$TARGET_BINARY_PATH")"
rm -rf "$EXTRACT_DIR"
mkdir -p "$EXTRACT_DIR"

CURL_ARGS=(-fsSL -H "Accept: application/octet-stream")
if [[ -n "$GITHUB_TOKEN" ]]; then
    CURL_ARGS+=(-H "Authorization: Bearer ${GITHUB_TOKEN}")
fi

curl "${CURL_ARGS[@]}" -o "$ARCHIVE_PATH" "$ASSET_API_URL"
tar -xzf "$ARCHIVE_PATH" -C "$EXTRACT_DIR"

EXTRACTED_BIN="$(find "$EXTRACT_DIR" -type f -name 'kalamdb-server-*linux-x86_64' | head -n1)"
if [[ -z "$EXTRACTED_BIN" ]]; then
    echo "Failed to locate kalamdb-server binary in ${ASSET_NAME}" >&2
    exit 1
fi

cp "$EXTRACTED_BIN" "$TARGET_BINARY_PATH"
chmod +x "$TARGET_BINARY_PATH"