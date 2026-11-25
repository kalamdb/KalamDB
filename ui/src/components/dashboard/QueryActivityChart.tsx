import { useMemo } from "react";
import { Area, AreaChart, CartesianGrid, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";
import { BarChart3 } from "lucide-react";
import { Card } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";
import type { DashboardMetricSample } from "@/services/systemTableService";
import { ACCENT, CHART_GRID, PANEL, chartTooltipStyle } from "./theme";

interface QueryActivityChartProps {
  data: DashboardMetricSample[];
  isLoading?: boolean;
  className?: string;
}

const RATE_KEYS = [
  "select_queries_per_second",
  "insert_queries_per_second",
  "update_queries_per_second",
  "delete_queries_per_second",
];

function formatTimeLabel(timestamp: number): string {
  return new Date(timestamp).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

function formatRate(value: number): string {
  return value.toLocaleString(undefined, { maximumFractionDigits: 2 });
}

export function totalQueryRate(sample: DashboardMetricSample): number {
  return RATE_KEYS.reduce((total, key) => {
    const value = Number(sample[key]);
    return Number.isFinite(value) ? total + value : total;
  }, 0);
}

export function QueryActivityChart({ data, isLoading, className }: QueryActivityChartProps) {
  const points = useMemo(
    () =>
      [...data]
        .sort((a, b) => a.sampled_at - b.sampled_at)
        .map((sample) => ({ timestamp: sample.sampled_at, rate: totalQueryRate(sample) })),
    [data],
  );

  return (
    <Card className={cn(PANEL, "gap-0 px-5 py-5", className)}>
      <div className="flex items-center gap-2">
        <span className="flex size-7 items-center justify-center rounded-lg bg-[var(--kdb-accent-soft)] text-[var(--kdb-accent-strong)]">
          <BarChart3 className="size-4" strokeWidth={2} />
        </span>
        <div>
          <p className="text-sm font-medium">SQL Query Activity</p>
          <p className="text-xs text-muted-foreground">Statements per second</p>
        </div>
      </div>

      <div className="mt-4 min-h-[240px] flex-1">
        {isLoading && points.length === 0 ? (
          <div className="flex h-full flex-col justify-end gap-2">
            <Skeleton className="h-full w-full rounded-lg" />
          </div>
        ) : points.length < 2 ? (
          <div className="flex h-full flex-col items-center justify-center gap-1 rounded-lg border border-dashed text-center">
            <p className="text-sm font-medium">Collecting samples</p>
            <p className="max-w-[34ch] text-xs text-muted-foreground">
              Query activity appears once the dashboard has polled the server a few times.
            </p>
          </div>
        ) : (
          <ResponsiveContainer width="100%" height="100%">
            <AreaChart data={points} margin={{ top: 8, right: 8, left: -18, bottom: 0 }}>
              <defs>
                <linearGradient id="kdb-query-fill" x1="0" y1="0" x2="0" y2="1">
                  <stop offset="0%" stopColor={ACCENT} stopOpacity={0.32} />
                  <stop offset="100%" stopColor={ACCENT} stopOpacity={0} />
                </linearGradient>
              </defs>
              <CartesianGrid stroke={CHART_GRID} strokeDasharray="3 3" vertical={false} />
              <XAxis
                dataKey="timestamp"
                type="number"
                domain={["dataMin", "dataMax"]}
                tickFormatter={formatTimeLabel}
                tick={{ fontSize: 11 }}
                tickMargin={8}
                minTickGap={40}
                stroke={CHART_GRID}
              />
              <YAxis
                tick={{ fontSize: 11 }}
                tickFormatter={(value) => formatRate(Number(value))}
                stroke={CHART_GRID}
                width={52}
              />
              <Tooltip
                contentStyle={chartTooltipStyle}
                formatter={(value) => [`${formatRate(Number(value))}/s`, "Statements"]}
                labelFormatter={(value) => formatTimeLabel(Number(value))}
              />
              <Area
                type="monotone"
                dataKey="rate"
                stroke={ACCENT}
                strokeWidth={2}
                fill="url(#kdb-query-fill)"
                activeDot={{ r: 3 }}
                connectNulls
              />
            </AreaChart>
          </ResponsiveContainer>
        )}
      </div>
    </Card>
  );
}
