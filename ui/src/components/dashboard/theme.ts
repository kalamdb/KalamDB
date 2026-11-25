export const ACCENT = "var(--kdb-accent)";
export const ACCENT_STRONG = "var(--kdb-accent-strong)";
export const CHART_GRID = "var(--kdb-grid)";
export const SERIES_MEMORY = "var(--kdb-series-memory)";
export const SERIES_FILES = "var(--kdb-series-files)";
export const VIOLET = "var(--kdb-violet)";

export const PANEL =
  "rounded-2xl bg-[var(--kdb-surface)] shadow-[0_1px_2px_rgb(0_0_0_/_0.03),0_18px_40px_-28px_rgb(0_0_0_/_0.22)]";

export const PANEL_LABEL =
  "text-[11px] font-semibold uppercase tracking-[0.16em] text-muted-foreground";

export const ACCENT_CHIP =
  "inline-flex items-center gap-1.5 rounded-full bg-[var(--kdb-accent-soft)] px-2.5 py-1 text-[11px] font-medium text-[var(--kdb-accent-strong)]";

export const ICON_CHIP =
  "flex size-8 items-center justify-center rounded-xl bg-[var(--kdb-accent-soft)] text-[var(--kdb-accent-strong)]";

export const ICON_CHIP_VIOLET =
  "flex size-8 items-center justify-center rounded-xl bg-[var(--kdb-violet-soft)] text-[var(--kdb-violet-strong)]";

export const chartTooltipStyle = {
  border: "1px solid var(--color-border)",
  borderRadius: "12px",
  background: "var(--color-popover)",
  fontSize: "12px",
  boxShadow: "0 10px 24px -16px rgb(0 0 0 / 0.45)",
};

export function enterAnimation(index: number) {
  return {
    className: "animate-in fade-in-0 slide-in-from-bottom-2 fill-mode-both duration-500",
    style: { animationDelay: `${index * 60}ms` },
  };
}

export function percentDelta(series: number[]): number | null {
  const finite = series.filter((value) => Number.isFinite(value));
  if (finite.length < 4) {
    return null;
  }

  const baseline = finite[0];
  const latest = finite[finite.length - 1];
  if (baseline === 0) {
    return latest === 0 ? 0 : null;
  }

  return ((latest - baseline) / Math.abs(baseline)) * 100;
}
