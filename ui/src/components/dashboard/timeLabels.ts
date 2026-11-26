const DAY_MS = 24 * 60 * 60 * 1000;

export interface TimeLabelFormatters {
  /** Axis ticks: clock time while the span fits in a day, calendar days beyond that. */
  axis: (timestamp: number) => string;
  /** Tooltips: always unambiguous, so multi-day spans carry the date too. */
  tooltip: (timestamp: number) => string;
}

function formatClock(timestamp: number): string {
  return new Date(timestamp).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

function formatDay(timestamp: number): string {
  return new Date(timestamp).toLocaleDateString([], { month: "short", day: "numeric" });
}

/**
 * Clock times repeat once a range covers more than a day, so "14:00" alone cannot
 * say which day holds a spike. Ranges wider than a day switch to dated labels.
 */
export function timeLabelFormatters(timestamps: number[]): TimeLabelFormatters {
  let earliest = Number.POSITIVE_INFINITY;
  let latest = Number.NEGATIVE_INFINITY;

  for (const timestamp of timestamps) {
    if (!Number.isFinite(timestamp)) {
      continue;
    }
    earliest = Math.min(earliest, timestamp);
    latest = Math.max(latest, timestamp);
  }

  const spansDays = Number.isFinite(earliest) && latest - earliest > DAY_MS;

  return {
    axis: (timestamp) => (spansDays ? formatDay(timestamp) : formatClock(timestamp)),
    tooltip: (timestamp) =>
      spansDays ? `${formatDay(timestamp)}, ${formatClock(timestamp)}` : formatClock(timestamp),
  };
}
