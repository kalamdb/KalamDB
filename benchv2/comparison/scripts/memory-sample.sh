#!/usr/bin/env bash
# Server memory samples for the comparison.
#
# On macOS the number that decides RAM pressure is phys_footprint, the same
# figure Activity Monitor and jetsam use. phys_footprint_peak is the kernel
# high-water mark since the process started, so a sample taken after the
# workload still contains the peak. Virtual size is not reported: SpaceTimeDB
# reserves a large page pool that is not resident until touched.

memory_footprint_bytes() {
  local pid=$1
  footprint -p "$pid" --format bytes --noCategories 2>/dev/null | awk '
    /phys_footprint_peak:/ { peak = $2; next }
    /phys_footprint:/ { current = $2 }
    END {
      if (current == "" || peak == "") exit 1
      printf "%s %s\n", current, peak
    }
  '
}

memory_mark_ready() {
  MEMORY_READY_CURRENT=0
  MEMORY_READY_PEAK=0
  if read -r MEMORY_READY_CURRENT MEMORY_READY_PEAK < <(memory_footprint_bytes "$1"); then
    return 0
  fi
  echo "memory sample failed for pid $1 before the workload" >&2
  return 0
}

# Wall clock from startup_begin (immediately before the server process is
# spawned) to startup_finish (the script's existing readiness probe). This
# excludes client compilation, harness schema bootstrap, and the timed workload.
startup_begin() {
  STARTUP_BEGIN_NS="$(python3 -c 'import time; print(time.time_ns())')"
}

startup_finish() {
  local end_ns
  end_ns="$(python3 -c 'import time; print(time.time_ns())')"
  STARTUP_SECONDS="$(
    STARTUP_BEGIN_NS="$STARTUP_BEGIN_NS" STARTUP_END_NS="$end_ns" python3 -c \
      'import os; b=int(os.environ["STARTUP_BEGIN_NS"]); e=int(os.environ["STARTUP_END_NS"]); print(f"{(e - b) / 1e9:.6f}")'
  )"
}

startup_header() {
  if [[ -n "${STARTUP_SECONDS:-}" ]]; then
    echo "# startup_seconds=${STARTUP_SECONDS}"
  fi
}

# COMPARISON_STARTUP_ONLY=1 records startup and stops before the workload.
startup_only_exit() {
  local track="$1"
  if [[ "${COMPARISON_STARTUP_ONLY:-}" != "1" ]]; then
    return 0
  fi
  local stamp out
  stamp="$(date +%Y%m%d-%H%M%S)"
  out="${RESULTS:?}/startup-${track}-${stamp}.txt"
  {
    echo "# track=${track}"
    echo "# startup_seconds=${STARTUP_SECONDS}"
    echo "# startup_definition=process spawn until the readiness probe, before client build, schema bootstrap, and the timed workload"
    echo "# measured_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  } >"$out"
  echo "Startup ${track} ${STARTUP_SECONDS}s"
  echo "Wrote ${out}"
  exit 0
}

memory_append_result() {
  local pid=$1
  local out=$2
  local end_current=0
  local end_peak=0
  if read -r end_current end_peak < <(memory_footprint_bytes "$pid"); then
    :
  else
    echo "memory sample failed for pid $pid after the workload" >&2
  fi
  {
    echo "# memory_tool=footprint"
    echo "# memory_metric=phys_footprint"
    echo "# memory_ready_bytes=${MEMORY_READY_CURRENT:-0}"
    echo "# memory_retained_bytes=${end_current}"
    echo "# memory_peak_bytes=${end_peak}"
  } | tee -a "$out"
}
