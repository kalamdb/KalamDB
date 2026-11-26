import { useMemo } from "react";
import { Area, AreaChart, CartesianGrid, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";
import { BarChart3 } from "lucide-react";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";
import type { DashboardMetricSample } from "@/services/systemTableService";
import { ACCENT, CHART_GRID, ICON_CHIP, PANEL, chartTooltipStyle } from "./theme";
import { timeLabelFormatters } from "./timeLabels";

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
  const timeLabels = useMemo(
    () => timeLabelFormatters(points.map((point) => point.timestamp)),
    [points],
  );

  return (
    <Card className={cn(PANEL, "gap-0 py-5", className)}>
      <CardHeader className="px-5">
        <CardTitle className="flex items-center gap-2 text-sm font-semibold">
          <span className={ICON_CHIP}>
            <BarChart3 className="size-4" strokeWidth={2} />
          </span>
          SQL Query Activity
        </CardTitle>
        <CardDescription className="pl-10 text-xs">SQL statements over time</CardDescription>
      </CardHeader>

      <CardContent className="mt-4 min-h-[240px] flex-1 px-5">
        {isLoading && points.length === 0 ? (
          <div className="flex h-full flex-col justify-end gap-2">
            <Skeleton className="h-full w-full rounded-lg" />
          </div>
        ) : points.length < 2 ? (
          <div className="flex h-full flex-col items-center justify-center gap-1 rounded-lg border border-dashed text-center">
            <p className="text-sm font-semibold">Collecting samples</p>
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
                tickFormatter={timeLabels.axis}
                tick={{ fontSize: 11, fontFamily: "var(--font-mono)" }}
                tickMargin={8}
                minTickGap={40}
                stroke={CHART_GRID}
              />
              <YAxis
                tick={{ fontSize: 11, fontFamily: "var(--font-mono)" }}
                tickFormatter={(value) => formatRate(Number(value))}
                stroke={CHART_GRID}
                width={52}
              />
              <Tooltip
                contentStyle={chartTooltipStyle}
                formatter={(value) => [`${formatRate(Number(value))}/s`, "Statements"]}
                labelFormatter={(value) => timeLabels.tooltip(Number(value))}
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
      </CardContent>
    </Card>
  );
}
