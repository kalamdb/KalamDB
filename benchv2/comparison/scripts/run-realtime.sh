#!/usr/bin/env bash
# Start each selected database on a fresh fixture and measure native subscriptions.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

if [[ "${1:-}" == "--client" ]]; then
  [[ $# == 3 ]] || exit 2
  system="$2"
  base="$3"
  out="$ROOT/results/realtime-${system}-$(date +%Y%m%d-%H%M%S).txt"
  {
    echo "# started=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "# server_build=${COMPARISON_SERVER_BUILD:-unrecorded}"
    if [[ "$system" == spacetimedb ]]; then
      COMPARISON_WORKLOAD=realtime SPACETIMEDB_URL="$base" "$ROOT/target/release/comparison_spacetimedb"
    else
      node "$ROOT/realtime/run.mjs" "$system" "$base"
    fi
    echo "# finished=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  } 2>&1 | tee "$out"
  echo "Wrote $out"
  exit 0
fi

if [[ $# == 0 ]]; then
  set -- kalamdb trailbase pocketbase surrealdb spacetimedb convex
fi
for system in "$@"; do
  case "$system" in
    kalamdb|trailbase|pocketbase|surrealdb|spacetimedb|convex) ;;
    *) echo "Unknown realtime system: $system" >&2; exit 2 ;;
  esac
done
for system in "$@"; do
  COMPARISON_WORKLOAD=realtime COMPARISON_STARTUP_ONLY=0 bash "$ROOT/scripts/run-$system.sh"
done
