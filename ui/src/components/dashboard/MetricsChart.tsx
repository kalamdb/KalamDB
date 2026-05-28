import { useMemo, type ReactNode } from "react";
import {
  AreaChart,
  Area,
  CartesianGrid,
  Legend,
  Line,
  LineChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { Card, CardHeader, CardTitle, CardContent } from "@/components/ui/card";
import type { DashboardMetricSample } from "@/services/systemTableService";

interface TimeSeriesData {
  time: string;
  timestamp: number;
  [key: string]: string | number;
}

interface MetricsChartProps {
  data: DashboardMetricSample[];
  isLoading?: boolean;
  trailingPanel?: ReactNode;
}

function formatTimeLabel(timestamp: number): string {
  const date = new Date(timestamp);
  return date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

function formatRateValue(value: number | string): string {
  const numeric = Number(value);
  if (!Number.isFinite(numeric)) return "0";
  return numeric.toLocaleString(undefined, { maximumFractionDigits: 2 });
}

function formatMetricTooltip(value: number | string, name: string | number | undefined): string {
  const formatted = formatRateValue(value);
  return String(name).includes("/s") ? `${formatted}/s` : formatted;
}

const tooltipStyle = {
  border: "none",
  borderRadius: "8px",
  boxShadow: "0 4px 6px -1px rgb(0 0 0 / 0.1)",
};

function ChartCard({ title, children }: { title: string; children: ReactNode }) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>{title}</CardTitle>
      </CardHeader>
      <CardContent className="h-80">{children}</CardContent>
    </Card>
  );
}

export function buildMetricChartData(data: DashboardMetricSample[]): TimeSeriesData[] {
  if (!data || data.length === 0) return [];

  return [...data]
    .filter((row) => Number.isFinite(row.sampled_at) && row.sampled_at > 0)
    .sort((a, b) => a.sampled_at - b.sampled_at)
    .map((row) => {
    const ts = Math.floor(row.sampled_at / 1000) * 1000;
    const point: TimeSeriesData = { time: formatTimeLabel(ts), timestamp: ts };

    for (const [key, value] of Object.entries(row)) {
      if (key !== "sampled_at" && typeof value === "number") {
        point[key] = value;
      }
    }

    return point;
  });
}

export function MetricsChart({ data, isLoading, trailingPanel }: MetricsChartProps) {
  const chartData = useMemo(() => buildMetricChartData(data), [data]);
  const gridClassName = "mt-6 grid grid-cols-1 gap-4 lg:grid-cols-2";

  if (isLoading) {
    return (
      <div className={gridClassName}>
        <Card className="h-96 flex items-center justify-center text-muted-foreground">
          Loading query activity...
        </Card>
        <Card className="h-96 flex items-center justify-center text-muted-foreground">
          Loading connection metrics...
        </Card>
        <Card className="h-96 flex items-center justify-center text-muted-foreground">
          Loading pub/sub metrics...
        </Card>
        <Card className="h-96 flex items-center justify-center text-muted-foreground">
          Loading resource metrics...
        </Card>
        <Card className="h-96 flex items-center justify-center text-muted-foreground">
          Loading storage metrics...
        </Card>
        {trailingPanel}
      </div>
    );
  }

  if (chartData.length === 0) {
    return (
      <div className={gridClassName}>
        <Card className="h-64 flex items-center justify-center text-muted-foreground">
          No historical metric data available yet.
        </Card>
        {trailingPanel}
      </div>
    );
  }

  return (
    <div className={gridClassName}>
      <ChartCard title="SQL Query Activity">
        <ResponsiveContainer width="100%" height="100%">
          <LineChart data={chartData} margin={{ top: 10, right: 30, left: 0, bottom: 0 }}>
            <CartesianGrid strokeDasharray="3 3" vertical={false} opacity={0.3} />
            <XAxis dataKey="timestamp" type="number" domain={["dataMin", "dataMax"]} tickFormatter={formatTimeLabel} tick={{ fontSize: 12 }} tickMargin={10} minTickGap={30} />
            <YAxis tick={{ fontSize: 12 }} tickFormatter={formatRateValue} />
            <Tooltip formatter={(value) => formatRateValue(value as number)} labelFormatter={(value) => formatTimeLabel(Number(value))} contentStyle={tooltipStyle} />
            <Legend wrapperStyle={{ paddingTop: "20px" }} />
            <Line type="step" dataKey="select_queries_per_second" name="Selects/s" stroke="#7c3aed" strokeWidth={2} dot={false} connectNulls />
            <Line type="step" dataKey="insert_queries_per_second" name="Inserts/s" stroke="#2563eb" strokeWidth={2} dot={false} connectNulls />
            <Line type="step" dataKey="update_queries_per_second" name="Updates/s" stroke="#f59e0b" strokeWidth={2} dot={false} connectNulls />
            <Line type="step" dataKey="delete_queries_per_second" name="Deletes/s" stroke="#dc2626" strokeWidth={2} dot={false} connectNulls />
          </LineChart>
        </ResponsiveContainer>
      </ChartCard>

      <ChartCard title="Connections & Subscriptions">
        <ResponsiveContainer width="100%" height="100%">
          <LineChart data={chartData} margin={{ top: 10, right: 30, left: 0, bottom: 0 }}>
            <CartesianGrid strokeDasharray="3 3" vertical={false} opacity={0.3} />
            <XAxis dataKey="timestamp" type="number" domain={["dataMin", "dataMax"]} tickFormatter={formatTimeLabel} tick={{ fontSize: 12 }} tickMargin={10} minTickGap={30} />
            <YAxis yAxisId="count" tick={{ fontSize: 12 }} allowDecimals={false} />
            <YAxis yAxisId="rate" orientation="right" tick={{ fontSize: 12 }} tickFormatter={formatRateValue} />
            <Tooltip formatter={(value, name) => formatMetricTooltip(value as number, name)} labelFormatter={(value) => formatTimeLabel(Number(value))} contentStyle={tooltipStyle} />
            <Legend wrapperStyle={{ paddingTop: "20px" }} />
            <Line yAxisId="count" type="monotone" dataKey="active_connections" name="Connections" stroke="#2563eb" strokeWidth={2} dot={false} connectNulls activeDot={{ r: 4 }} />
            <Line yAxisId="count" type="step" dataKey="active_connections_peak" name="Peak Connections" stroke="#dc2626" strokeDasharray="5 5" strokeWidth={2} dot={false} connectNulls />
            <Line yAxisId="count" type="monotone" dataKey="active_subscriptions" name="Subscriptions" stroke="#16a34a" strokeWidth={2} dot={false} connectNulls activeDot={{ r: 4 }} />
            <Line yAxisId="count" type="step" dataKey="active_subscriptions_peak" name="Peak Subscriptions" stroke="#f59e0b" strokeDasharray="5 5" strokeWidth={2} dot={false} connectNulls />
            <Line yAxisId="rate" type="step" dataKey="subscription_changes_delivered_per_second" name="Changes/s" stroke="#0891b2" strokeWidth={2} dot={false} connectNulls />
          </LineChart>
        </ResponsiveContainer>
      </ChartCard>

      <ChartCard title="Pub/Sub Consumers">
        <ResponsiveContainer width="100%" height="100%">
          <LineChart data={chartData} margin={{ top: 10, right: 30, left: 0, bottom: 0 }}>
            <CartesianGrid strokeDasharray="3 3" vertical={false} opacity={0.3} />
            <XAxis dataKey="timestamp" type="number" domain={["dataMin", "dataMax"]} tickFormatter={formatTimeLabel} tick={{ fontSize: 12 }} tickMargin={10} minTickGap={30} />
            <YAxis yAxisId="count" tick={{ fontSize: 12 }} allowDecimals={false} />
            <YAxis yAxisId="rate" orientation="right" tick={{ fontSize: 12 }} tickFormatter={formatRateValue} />
            <Tooltip formatter={(value, name) => formatMetricTooltip(value as number, name)} labelFormatter={(value) => formatTimeLabel(Number(value))} contentStyle={tooltipStyle} />
            <Legend wrapperStyle={{ paddingTop: "20px" }} />
            <Line yAxisId="count" type="monotone" dataKey="pubsub_active_consumers" name="Active Consumers" stroke="#2563eb" strokeWidth={2} dot={false} connectNulls activeDot={{ r: 4 }} />
            <Line yAxisId="rate" type="step" dataKey="pubsub_messages_consumed_per_second" name="Msg/s" stroke="#16a34a" strokeWidth={2} dot={false} connectNulls />
            <Line yAxisId="rate" type="step" dataKey="pubsub_messages_consumed_peak_per_second" name="Peak Msg/s" stroke="#f59e0b" strokeDasharray="5 5" strokeWidth={2} dot={false} connectNulls />
            <Line yAxisId="rate" type="step" dataKey="pubsub_kb_consumed_per_second" name="KB/s" stroke="#7c3aed" strokeWidth={2} dot={false} connectNulls />
            <Line yAxisId="count" type="monotone" dataKey="topic_cache_topic_count" name="Total Topics" stroke="#dc2626" strokeDasharray="6 4" strokeWidth={2} dot={false} connectNulls />
          </LineChart>
        </ResponsiveContainer>
      </ChartCard>

      <ChartCard title="Resource Usage">
        <ResponsiveContainer width="100%" height="100%">
          <AreaChart data={chartData} margin={{ top: 10, right: 30, left: 0, bottom: 0 }}>
            <CartesianGrid strokeDasharray="3 3" vertical={false} opacity={0.3} />
            <XAxis dataKey="timestamp" type="number" domain={["dataMin", "dataMax"]} tickFormatter={formatTimeLabel} tick={{ fontSize: 12 }} tickMargin={10} minTickGap={30} />
            <YAxis yAxisId="memory" tick={{ fontSize: 12 }} />
            <YAxis yAxisId="cpu" orientation="right" tick={{ fontSize: 12 }} domain={[0, 100]} />
            <YAxis yAxisId="files" orientation="right" hide />
            <Tooltip labelFormatter={(value) => formatTimeLabel(Number(value))} contentStyle={tooltipStyle} />
            <Legend wrapperStyle={{ paddingTop: "20px" }} />
            <Area yAxisId="memory" type="monotone" dataKey="memory_usage_mb" name="Memory (MB)" stroke="#7c3aed" fill="#7c3aed" fillOpacity={0.16} connectNulls />
            <Area yAxisId="cpu" type="monotone" dataKey="cpu_usage_percent" name="CPU (%)" stroke="#f97316" fill="#f97316" fillOpacity={0.16} connectNulls />
            <Line yAxisId="files" type="monotone" dataKey="open_files_total" name="Files Total" stroke="#0891b2" strokeWidth={2} dot={false} connectNulls />
            <Line yAxisId="files" type="monotone" dataKey="open_files_directories" name="File Directories" stroke="#16a34a" strokeWidth={2} dot={false} connectNulls />
          </AreaChart>
        </ResponsiveContainer>
      </ChartCard>

      <ChartCard title="Manifest & Parquet Rates">
        <ResponsiveContainer width="100%" height="100%">
          <LineChart data={chartData} margin={{ top: 10, right: 30, left: 0, bottom: 0 }}>
            <CartesianGrid strokeDasharray="3 3" vertical={false} opacity={0.3} />
            <XAxis dataKey="timestamp" type="number" domain={["dataMin", "dataMax"]} tickFormatter={formatTimeLabel} tick={{ fontSize: 12 }} tickMargin={10} minTickGap={30} />
            <YAxis yAxisId="count" tick={{ fontSize: 12 }} allowDecimals={false} />
            <YAxis yAxisId="rate" orientation="right" tick={{ fontSize: 12 }} tickFormatter={formatRateValue} />
            <Tooltip formatter={(value, name) => name === "Manifest Count" ? formatRateValue(value as number) : `${formatRateValue(value as number)}/s`} labelFormatter={(value) => formatTimeLabel(Number(value))} contentStyle={tooltipStyle} />
            <Legend wrapperStyle={{ paddingTop: "20px" }} />
            <Line yAxisId="count" type="step" dataKey="manifest_cache_rocksdb_entries" name="Manifest Count" stroke="#2563eb" strokeWidth={2} dot={false} connectNulls />
            <Line yAxisId="rate" type="step" dataKey="manifest_writes_per_second" name="Manifest Writes/s" stroke="#16a34a" strokeWidth={2} dot={false} connectNulls />
            <Line yAxisId="rate" type="step" dataKey="manifest_reads_per_second" name="Manifest Reads/s" stroke="#f59e0b" strokeWidth={2} dot={false} connectNulls />
            <Line yAxisId="rate" type="step" dataKey="parquet_files_written_per_second" name="Parquet Written/s" stroke="#7c3aed" strokeDasharray="6 4" strokeWidth={2} dot={false} connectNulls />
            <Line yAxisId="rate" type="step" dataKey="parquet_files_read_per_second" name="Parquet Read/s" stroke="#dc2626" strokeDasharray="6 4" strokeWidth={2} dot={false} connectNulls />
          </LineChart>
        </ResponsiveContainer>
      </ChartCard>

      {trailingPanel}
    </div>
  );
}
