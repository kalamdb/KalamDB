import { Cpu } from "lucide-react";
import { Card } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";
import type { DashboardMetricSample } from "@/services/systemTableService";
import { Sparkline } from "./Sparkline";
import { PANEL, enterAnimation } from "./theme";

interface ResourcePanelProps {
  samples: DashboardMetricSample[];
  isLoading?: boolean;
  className?: string;
  index?: number;
}

const ROWS = [
  { key: "cpu_usage_percent", label: "CPU", unit: "%", digits: 1 },
  { key: "memory_usage_mb", label: "Memory", unit: " MB", digits: 0 },
  { key: "open_files_total", label: "Open files", unit: "", digits: 0 },
];

function seriesFor(samples: DashboardMetricSample[], key: string): number[] {
  return samples
    .map((sample) => Number(sample[key]))
    .filter((value) => Number.isFinite(value));
}

export function ResourcePanel({ samples, isLoading, className, index = 0 }: ResourcePanelProps) {
  const entrance = enterAnimation(index);

  return (
    <Card
      className={cn(PANEL, "gap-0 px-5 py-4", entrance.className, className)}
      style={entrance.style}
    >
      <div className="flex items-center gap-2">
        <span className="flex size-7 items-center justify-center rounded-lg bg-[var(--kdb-accent-soft)] text-[var(--kdb-accent-strong)]">
          <Cpu className="size-4" strokeWidth={2} />
        </span>
        <div>
          <p className="text-sm font-medium">Resource Usage</p>
          <p className="text-xs text-muted-foreground">Process CPU, memory and file handles</p>
        </div>
      </div>

      {isLoading && samples.length === 0 ? (
        <div className="mt-4 space-y-3">
          {ROWS.map((row) => (
            <Skeleton key={row.key} className="h-9" />
          ))}
        </div>
      ) : (
        <div className="mt-4 divide-y divide-border/70">
          {ROWS.map((row) => {
            const series = seriesFor(samples, row.key);
            const latest = series.length > 0 ? series[series.length - 1] : null;

            return (
              <div key={row.key} className="flex items-center gap-4 py-2.5 first:pt-0 last:pb-0">
                <span className="w-24 shrink-0 text-xs text-muted-foreground">{row.label}</span>
                <Sparkline values={series} className="min-w-0 flex-1" height={26} />
                <span className="w-24 shrink-0 text-right text-sm font-semibold tabular-nums">
                  {latest === null
                    ? "—"
                    : `${latest.toLocaleString(undefined, {
                        minimumFractionDigits: row.digits,
                        maximumFractionDigits: row.digits,
                      })}${row.unit}`}
                </span>
              </div>
            );
          })}
        </div>
      )}
    </Card>
  );
}
