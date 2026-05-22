#!/bin/bash
set -euo pipefail

echo "🧪 Testing KalamDB TypeScript Consumer SDK..."

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

echo "📥 Ensuring npm dependencies are installed..."
npm install --no-audit --no-fund

echo "🔬 Running consumer package tests..."
npm test

echo ""
echo "✅ All TypeScript consumer SDK tests passed!"