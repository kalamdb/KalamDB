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
