import { useMemo } from "react";
import { Area, AreaChart, CartesianGrid, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";
import { Cpu } from "lucide-react";
import { Card } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";
import type { DashboardMetricSample } from "@/services/systemTableService";
import {
  ACCENT,
  CHART_GRID,
  ICON_CHIP,
  PANEL,
  SERIES_FILES,
  SERIES_MEMORY,
  chartTooltipStyle,
  enterAnimation,
} from "./theme";
import { timeLabelFormatters } from "./timeLabels";

interface ResourcePanelProps {
  samples: DashboardMetricSample[];
  isLoading?: boolean;
  className?: string;
  index?: number;
}

const SERIES = [
  { key: "cpu_usage_percent", label: "CPU", color: ACCENT, unit: "%", digits: 1, axis: "pct" },
  { key: "memory_usage_mb", label: "Memory", color: SERIES_MEMORY, unit: " MB", digits: 0, axis: "abs" },
  { key: "open_files_total", label: "Open files", color: SERIES_FILES, unit: "", digits: 0, axis: "abs" },
];

export function ResourcePanel({ samples, isLoading, className, index = 0 }: ResourcePanelProps) {
  const entrance = enterAnimation(index);

  const points = useMemo(
    () =>
      [...samples]
        .sort((a, b) => a.sampled_at - b.sampled_at)
        .map((sample) => {
          const point: Record<string, number> = { timestamp: sample.sampled_at };
          for (const series of SERIES) {
            const value = Number(sample[series.key]);
            if (Number.isFinite(value)) {
              point[series.key] = value;
            }
          }
          return point;
        }),
    [samples],
  );
  const timeLabels = useMemo(
    () => timeLabelFormatters(points.map((point) => point.timestamp)),
    [points],
  );

  const latest = points.length > 0 ? points[points.length - 1] : undefined;

  return (
    <Card
      className={cn(PANEL, "gap-0 overflow-hidden px-5 py-4", entrance.className, className)}
      style={entrance.style}
    >
      <div className="flex items-center gap-2.5">
        <span className={ICON_CHIP}>
          <Cpu className="size-4" strokeWidth={2} />
        </span>
        <div>
          <p className="text-sm font-semibold">Resource Usage</p>
          <p className="text-xs text-muted-foreground">CPU, memory and file handles</p>
        </div>
      </div>

      {isLoading && points.length === 0 ? (
        <div className="mt-4 space-y-3">
          <Skeleton className="h-16 w-32" />
          <Skeleton className="h-28 w-full" />
        </div>
      ) : (
        <div className="mt-4 grid gap-3 sm:grid-cols-[minmax(0,auto)_minmax(0,1fr)]">
          <dl className="space-y-2.5">
            {SERIES.map((series) => {
              const value = latest?.[series.key];
              return (
                <div key={series.key} className="flex items-center gap-2.5 text-xs">
                  <span
                    className="size-2.5 shrink-0 rounded-full"
                    style={{ backgroundColor: series.color }}
                  />
                  <dt className="w-[72px] text-muted-foreground">{series.label}</dt>
                  <dd className="font-semibold tabular-nums">
                    {value === undefined
                      ? "—"
                      : `${value.toLocaleString(undefined, {
                          minimumFractionDigits: series.digits,
                          maximumFractionDigits: series.digits,
                        })}${series.unit}`}
                  </dd>
                </div>
              );
            })}
          </dl>

          <div className="h-[132px] min-w-0">
            {points.length < 2 ? (
              <div className="flex h-full items-center justify-center rounded-lg border border-dashed text-center text-xs text-muted-foreground">
                Collecting samples
              </div>
            ) : (
              <ResponsiveContainer width="100%" height="100%">
                <AreaChart data={points} margin={{ top: 12, right: 6, left: -16, bottom: 0 }}>
                  <defs>
                    {SERIES.map((series) => (
                      <linearGradient key={series.key} id={`kdb-res-${series.key}`} x1="0" y1="0" x2="0" y2="1">
                        <stop offset="0%" stopColor={series.color} stopOpacity={0.26} />
                        <stop offset="100%" stopColor={series.color} stopOpacity={0} />
                      </linearGradient>
                    ))}
                  </defs>
                  <CartesianGrid stroke={CHART_GRID} strokeDasharray="3 3" vertical={false} />
                  <XAxis
                    dataKey="timestamp"
                    type="number"
                    domain={["dataMin", "dataMax"]}
                    tickFormatter={timeLabels.axis}
                    tick={{ fontSize: 10 }}
                    minTickGap={44}
                    stroke={CHART_GRID}
                  />
                  <YAxis
                    yAxisId="pct"
                    domain={[0, 100]}
                    ticks={[0, 25, 50, 75, 100]}
                    tickFormatter={(value) => `${value}%`}
                    tick={{ fontSize: 10 }}
                    stroke={CHART_GRID}
                    width={52}
                  />
                  <YAxis yAxisId="abs" orientation="right" hide />
                  <Tooltip
                    contentStyle={chartTooltipStyle}
                    labelFormatter={(value) => timeLabels.tooltip(Number(value))}
                  />
                  {SERIES.map((series) => (
                    <Area
                      key={series.key}
                      yAxisId={series.axis}
                      type="monotone"
                      dataKey={series.key}
                      name={series.label}
                      stroke={series.color}
                      strokeWidth={1.75}
                      fill={`url(#kdb-res-${series.key})`}
                      connectNulls
                      dot={false}
                    />
                  ))}
                </AreaChart>
              </ResponsiveContainer>
            )}
          </div>
        </div>
      )}
    </Card>
  );
}
