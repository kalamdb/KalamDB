#!/usr/bin/env bash
# publish.sh — Publish kalam_sync to pub.dev
# Usage: ./publish.sh [--dry-run]
set -euo pipefail

DRY_RUN=false
for arg in "$@"; do
  case "$arg" in
    --dry-run) DRY_RUN=true ;;
  esac
done

cd "$(dirname "$0")"

echo "==> Checking git working tree..."
if [[ -n "$(git status --porcelain -- .)" ]]; then
  echo "ERROR: Uncommitted changes detected in this package directory."
  echo "Commit or stash them before publishing."
  git status --short -- .
  exit 1
fi

echo "==> Running flutter pub get..."
flutter pub get

echo "==> Running dart analyze..."
dart analyze lib

echo "==> Running tests..."
flutter test || true   # warn but don't block (e2e tests may require a running server)

if $DRY_RUN; then
  echo "==> Dry run — validating package (no upload)..."
  flutter pub publish --dry-run
  echo ""
  echo "Dry run complete. Run './publish.sh' (without --dry-run) to publish."
else
  echo "==> Publishing kalam_sync to pub.dev..."
  echo "y" | flutter pub publish
fi
