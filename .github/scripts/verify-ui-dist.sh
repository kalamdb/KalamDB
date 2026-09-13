#!/usr/bin/env bash
# Fail the release if ui/dist is missing or still the debug placeholder.
set -euo pipefail

index="${1:-ui/dist/index.html}"

if [[ ! -f "$index" ]]; then
  echo "missing $index — Admin UI dist was not downloaded or built" >&2
  exit 1
fi

if grep -Fq "UI Not Built" "$index"; then
  echo "$index is the debug placeholder, not a built Admin UI" >&2
  exit 1
fi

if ! grep -Fq 'id="root"' "$index"; then
  echo "$index does not look like the Vite Admin UI (missing id=\"root\")" >&2
  exit 1
fi
