#!/usr/bin/env bash
# Run the PostgreSQL side of the pgwire comparison as a native process.
#
# Default: initdb + pg_ctl a throwaway PostgreSQL 18 cluster under
# setups/postgres (same machine as kalamdb-server; not Docker). Set
# POSTGRES_URL to skip startup and use an existing server.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SETUP="$ROOT/setups/postgres"
DATA="$SETUP/data"
LOGS="$SETUP/logs"
RESULTS="$ROOT/results"
PORT="${POSTGRES_PORT:-25433}"
STARTED_SERVER=0
PG_BINDIR=""
PG_VERSION="existing"

mkdir -p "$RESULTS" "$SETUP"

shared_buffers_mb() {
  local ram_bytes=""
  if ram_bytes=$(sysctl -n hw.memsize 2>/dev/null); then
    :
  elif ram_bytes=$(awk '/MemTotal/ {print $2 * 1024}' /proc/meminfo 2>/dev/null); then
    :
  else
    echo "${SHARED_BUFFERS_MB:-256}"
    return
  fi
  local mb=$((ram_bytes / 8 / 1024 / 1024))
  if ((mb < 256)); then mb=256; fi
  if ((mb > 4096)); then mb=4096; fi
  echo "${SHARED_BUFFERS_MB:-$mb}"
}

resolve_pg_bindir() {
  if [[ -n "${POSTGRES_BIN:-}" ]]; then
    echo "$POSTGRES_BIN"
    return
  fi
  local prefix=""
  if command -v brew >/dev/null; then
    prefix="$(brew --prefix postgresql@18 2>/dev/null || true)"
    if [[ -x "${prefix:-}/bin/postgres" ]]; then
      echo "$prefix/bin"
      return
    fi
  fi
  for candidate in \
    /opt/homebrew/opt/postgresql@18/bin \
    /usr/local/opt/postgresql@18/bin \
    /usr/lib/postgresql/18/bin
  do
    if [[ -x "$candidate/postgres" ]]; then
      echo "$candidate"
      return
    fi
  done
  return 1
}

ensure_homebrew_runtime_paths() {
  # postgresql@18 is keg-only. When `brew link` is skipped (libpq already owns
  # client tools), postgres still looks under HOMEBREW_PREFIX/{share,lib}/postgresql@18.
  local prefix=""
  if command -v brew >/dev/null; then
    prefix="$(brew --prefix 2>/dev/null || true)"
  fi
  prefix="${prefix:-/opt/homebrew}"
  local keg
  keg="$(dirname "$PG_BINDIR")"
  local share_src="$keg/share/postgresql"
  local lib_src="$keg/lib/postgresql"
  local share_dst="$prefix/share/postgresql@18"
  local lib_dst="$prefix/lib/postgresql@18"

  if [[ -d "$share_src" && ! -e "$share_dst" ]]; then
    mkdir -p "$(dirname "$share_dst")"
    ln -s "$share_src" "$share_dst"
  fi
  if [[ -d "$lib_src" && ! -e "$lib_dst" ]]; then
    mkdir -p "$(dirname "$lib_dst")"
    ln -s "$lib_src" "$lib_dst"
  fi
}

ensure_pg18() {
  if resolve_pg_bindir >/dev/null; then
    return
  fi
  command -v brew >/dev/null || {
    echo "PostgreSQL 18 server binaries not found. Install postgresql@18 or set POSTGRES_BIN" >&2
    exit 1
  }
  echo "postgresql@18 not installed — brew install postgresql@18"
  brew install postgresql@18
}

cleanup() {
  if [[ "$STARTED_SERVER" -eq 1 && -n "$PG_BINDIR" ]]; then
    "$PG_BINDIR/pg_ctl" -D "$DATA" -m fast stop >/dev/null 2>&1 || true
  fi
}

if [[ -n "${POSTGRES_URL:-}" ]]; then
  echo "Using existing POSTGRES_URL (native server not started)"
else
  ensure_pg18
  PG_BINDIR="$(resolve_pg_bindir)" || {
    echo "PostgreSQL 18 server binaries not found after install — set POSTGRES_BIN" >&2
    exit 1
  }
  [[ -x "$PG_BINDIR/postgres" && -x "$PG_BINDIR/pg_ctl" && -x "$PG_BINDIR/initdb" ]] || {
    echo "POSTGRES_BIN=$PG_BINDIR is missing postgres/pg_ctl/initdb (libpq client tools are not enough)" >&2
    exit 1
  }
  ensure_homebrew_runtime_paths

  if command -v docker >/dev/null && docker ps --format '{{.Names}}' 2>/dev/null | grep -qx kalam-vs-pg-postgres; then
    echo "Stopping leftover Docker comparison container kalam-vs-pg-postgres"
    docker rm -f kalam-vs-pg-postgres >/dev/null
  fi

  if (echo >/dev/tcp/127.0.0.1/"$PORT") >/dev/null 2>&1; then
    echo "Port ${PORT} is already in use — set POSTGRES_PORT or POSTGRES_URL" >&2
    exit 1
  fi

  SHARED_MB="$(shared_buffers_mb)"
  PG_VERSION="$("$PG_BINDIR/postgres" --version)"
  SHAREDIR="$("$PG_BINDIR/pg_config" --sharedir)"
  if [[ ! -f "$SHAREDIR/postgres.bki" ]]; then
    for candidate in \
      "$(dirname "$PG_BINDIR")/share/postgresql" \
      "$(dirname "$PG_BINDIR")/share/postgresql@18"
    do
      if [[ -f "$candidate/postgres.bki" ]]; then
        SHAREDIR="$candidate"
        break
      fi
    done
  fi
  [[ -f "$SHAREDIR/postgres.bki" ]] || {
    echo "initdb input files not found (pg_config --sharedir=$SHAREDIR). Homebrew postgresql@18 is keg-only; this script looks next to POSTGRES_BIN." >&2
    exit 1
  }

  rm -rf "$DATA" "$LOGS"
  mkdir -p "$DATA" "$LOGS"

  "$PG_BINDIR/initdb" \
    --pgdata="$DATA" \
    --username=postgres \
    --auth=trust \
    --encoding=UTF8 \
    --locale=C \
    --no-instructions \
    -L "$SHAREDIR" \
    >/dev/null

  trap cleanup EXIT
  "$PG_BINDIR/pg_ctl" -D "$DATA" -l "$LOGS/postgres.log" -o "\
-p ${PORT} -c listen_addresses=127.0.0.1 -c unix_socket_directories=${SETUP} \
-c shared_buffers=${SHARED_MB}MB -c effective_cache_size=$((SHARED_MB * 3))MB \
-c work_mem=4MB -c maintenance_work_mem=64MB \
-c synchronous_commit=off -c fsync=on -c full_page_writes=on \
-c max_connections=100 -c wal_compression=on \
-c checkpoint_timeout=15min -c max_wal_size=2GB" start >/dev/null
  STARTED_SERVER=1

  for _ in $(seq 1 60); do
    if "$PG_BINDIR/pg_isready" -h 127.0.0.1 -p "$PORT" -U postgres >/dev/null 2>&1; then
      break
    fi
    sleep 0.25
  done
  "$PG_BINDIR/pg_isready" -h 127.0.0.1 -p "$PORT" -U postgres >/dev/null
  "$PG_BINDIR/createdb" -h 127.0.0.1 -p "$PORT" -U postgres bench

  export POSTGRES_URL="postgres://postgres:postgres@127.0.0.1:${PORT}/bench"
  echo "Started native ${PG_VERSION} on 127.0.0.1:${PORT} (bindir=${PG_BINDIR}, shared_buffers=${SHARED_MB}MB, synchronous_commit=off)"
fi

cd "$ROOT"
cargo build --release --bin kalam_vs_pg_postgres
OUT="$RESULTS/postgres-$(date +%Y%m%d-%H%M%S).txt"
{
  echo "# KalamDB vs PostgreSQL — PostgreSQL (native pgwire)"
  echo "# postgres_bindir=${PG_BINDIR:-existing}"
  echo "# postgres_version=${PG_VERSION}"
  echo "# postgres_url=${POSTGRES_URL}"
  echo "# protocol=pgwire prepared statements; pool=16"
  echo "# table=heap + btree primary key (not UNLOGGED, not Docker)"
  echo "# durability=synchronous_commit=off, fsync=on, WAL on"
  echo "# started=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  POSTGRES_URL="$POSTGRES_URL" ./target/release/kalam_vs_pg_postgres
  echo "# finished=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} | tee "$OUT"

echo "Wrote $OUT"
