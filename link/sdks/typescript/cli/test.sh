#!/bin/bash
set -euo pipefail

echo "🧪 Testing KalamDB TypeScript CLI package..."

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

echo "📥 Ensuring npm dependencies are installed..."
npm install --foreground-scripts --no-audit --no-fund

if [[ ! -x "dist/kalam" && ! -x "dist/kalam.exe" ]]; then
	echo "❌ npm install completed without installing the kalam binary in dist/." >&2
	exit 1
fi

echo "🔬 Running CLI package tests..."
npm test

if [[ "${KALAM_CLI_SKIP_RELEASE_ASSET_CHECK:-false}" != "true" ]]; then
	echo "🔗 Validating release asset links and SHA256SUMS entries..."
	node scripts/validate-release-assets.js
else
	echo "⏭️  Skipping release asset validation (KALAM_CLI_SKIP_RELEASE_ASSET_CHECK=true)"
fi

echo "📦 Validating npm pack output..."
rm -rf dist
npm pack --dry-run >/dev/null

echo ""
echo "✅ All TypeScript CLI package tests passed!"