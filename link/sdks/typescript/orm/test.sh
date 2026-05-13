#!/bin/bash
set -euo pipefail

echo "🧪 Testing KalamDB TypeScript ORM SDK..."

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

export KALAMDB_TEST_URL="${KALAMDB_TEST_URL:-${KALAMDB_URL:-http://localhost:2900}}"
export KALAMDB_TEST_USER="${KALAMDB_TEST_USER:-${KALAMDB_USER:-admin}}"
export KALAMDB_TEST_PASSWORD="${KALAMDB_TEST_PASSWORD:-${KALAMDB_PASSWORD:-kalamdb123}}"

echo "📥 Ensuring npm dependencies are installed..."
npm install --no-audit --no-fund

echo "🔬 Running ORM package tests..."
npm test

echo ""
echo "✅ All TypeScript ORM SDK tests passed!"