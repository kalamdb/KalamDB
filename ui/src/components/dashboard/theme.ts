export const ACCENT = "var(--kdb-accent)";
export const ACCENT_STRONG = "var(--kdb-accent-strong)";
export const CHART_GRID = "var(--kdb-grid)";

export const PANEL =
  "rounded-xl shadow-[0_1px_2px_rgb(0_0_0_/_0.04),0_14px_30px_-22px_rgb(0_0_0_/_0.28)]";

export const PANEL_LABEL =
  "text-[11px] font-semibold uppercase tracking-[0.16em] text-muted-foreground";

export const ACCENT_CHIP =
  "inline-flex items-center gap-1.5 rounded-full bg-[var(--kdb-accent-soft)] px-2.5 py-1 text-[11px] font-medium text-[var(--kdb-accent-strong)]";

export const chartTooltipStyle = {
  border: "1px solid var(--color-border)",
  borderRadius: "10px",
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
