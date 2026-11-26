import { ArrowDown, ArrowUp, type LucideIcon } from "lucide-react";
import { Progress } from "@/components/ui/progress";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";
import { NUMERIC } from "./theme";

export interface HeroMetric {
  label: string;
  value: string;
  icon: LucideIcon;
  /** Secondary line under the figure; omitted when a progress bar is shown. */
  detail?: string;
  denominator?: string;
  delta?: number | null;
  deltaGoodWhen?: "up" | "down";
  progress?: { percent: number; label: string };
}

interface HeroMetricStripProps {
  metrics: HeroMetric[];
  isLoading?: boolean;
}

function DeltaBadge({ delta, goodWhen }: { delta: number; goodWhen: "up" | "down" }) {
  const isUp = delta >= 0;
  const isGood = goodWhen === "up" ? isUp : !isUp;
  const Icon = isUp ? ArrowUp : ArrowDown;

  return (
    <span
      className={cn(
        "inline-flex shrink-0 items-center gap-0.5 rounded-full px-1.5 py-0.5 text-[11px] font-semibold tabular-nums",
        isGood
          ? "bg-[var(--kdb-accent-soft)] text-[var(--kdb-accent-strong)]"
          : "bg-destructive/10 text-destructive",
      )}
    >
      <Icon className="size-3" strokeWidth={2.5} />
      {isUp ? "+" : "−"}
      {Math.abs(delta).toFixed(Math.abs(delta) >= 10 ? 0 : 1)}%
    </span>
  );
}

/**
 * Metrics read as one band rather than four boxes: the hero already provides the
 * surface, so these only need dividers to separate them.
 */
export function HeroMetricStrip({ metrics, isLoading }: HeroMetricStripProps) {
  return (
    <div className="relative grid gap-px border-t bg-border/60 sm:grid-cols-2 xl:grid-cols-4">
      {metrics.map((metric) => (
        <div key={metric.label} className="bg-[var(--kdb-strip)] px-5 py-4 sm:px-6">
          <div className="flex items-center gap-2">
            <metric.icon className="size-4 shrink-0 text-[var(--kdb-accent-strong)]" strokeWidth={2} />
            <span className="truncate text-sm font-medium text-muted-foreground">{metric.label}</span>
          </div>

          {isLoading ? (
            <div className="mt-2.5 space-y-2">
              <Skeleton className="h-7 w-24" />
              <Skeleton className="h-3 w-28" />
            </div>
          ) : (
            <>
              <div className="mt-2 flex items-baseline gap-2">
                <p className={cn("min-w-0 truncate text-[1.6rem] font-semibold leading-none", NUMERIC)}>
                  {metric.value}
                  {metric.denominator && (
                    <span className="ml-1 text-base font-medium text-muted-foreground">
                      {metric.denominator}
                    </span>
                  )}
                </p>
                {metric.delta !== null && metric.delta !== undefined && (
                  <DeltaBadge delta={metric.delta} goodWhen={metric.deltaGoodWhen ?? "up"} />
                )}
              </div>

              {metric.progress ? (
                <div className="mt-3 flex items-center gap-2.5">
                  <Progress
                    value={metric.progress.percent}
                    className="h-2 [&_[data-slot=progress-indicator]]:bg-[var(--kdb-accent)]"
                  />
                  <span className={cn("shrink-0 text-xs font-semibold text-muted-foreground", NUMERIC)}>
                    {metric.progress.label}
                  </span>
                </div>
              ) : (
                <p className="mt-1.5 truncate text-xs text-muted-foreground">{metric.detail ?? ""}</p>
              )}
            </>
          )}
        </div>
      ))}
    </div>
  );
}
