#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

paths=(
  README.md
  backend/README.md
  backend/server.example.toml
  server.toml
  docs
)

patterns=(
  '^\s*\[oauth(\.|\]|$)'
  '^\s*\[authentication\]'
  'oauth\.providers'
  '/auth/oauth/providers'
  'provider_family|provider\.family'
  'auto_create_users_from_provider'
)

failed=0
for pattern in "${patterns[@]}"; do
  if rg -n --hidden --glob '!docs/archive/**' --glob '!target/**' --glob '!**/node_modules/**' "$pattern" "${paths[@]}"; then
    failed=1
  fi
done

if [[ "$failed" -ne 0 ]]; then
  echo "Old auth provider configuration references remain in docs/configuration files." >&2
  exit 1
fi

echo "Auth docs/config cleanup guard passed."
