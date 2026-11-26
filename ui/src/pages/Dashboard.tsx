import { useEffect, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import {
  Clock3,
  Database,
  DatabaseZap,
  Gauge,
  MessageSquare,
  Plug,
  RefreshCw,
  Timer,
  Wifi,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { DashboardClusterOverview } from "@/components/dashboard/ClusterOverview";
import { DashboardHero } from "@/components/dashboard/DashboardHero";
import { MetricsChart } from "@/components/dashboard/MetricsChart";
import { QueryActivityChart } from "@/components/dashboard/QueryActivityChart";
import { ResourcePanel } from "@/components/dashboard/ResourcePanel";
import { SlowQueriesPanel } from "@/components/dashboard/SlowQueriesPanel";
import { StatGroupPanel, type StatGroupItem } from "@/components/dashboard/StatGroupPanel";
import type { HeroMetric } from "@/components/dashboard/HeroMetricStrip";
import { StorageUsageChart } from "@/components/dashboard/StorageUsageChart";
import { ACCENT_CHIP, PANEL_LABEL, percentDelta } from "@/components/dashboard/theme";
import { useAuth } from "@/lib/auth";
import {
  getTimeRangeCutoff,
  statsMapToDashboardSample,
  type DashboardMetricSample,
  type SystemStatsMap,
} from "@/services/systemTableService";
import {
  useCheckStorageHealthMutation,
  useGetClusterSnapshotQuery,
  useGetSlowQueriesQuery,
  useGetStatsQuery,
  useGetStoragesQuery,
} from "@/store/apiSlice";

const EMPTY_STATS: SystemStatsMap = {};
const DASHBOARD_STATS_POLL_INTERVAL_MS = 5000;
const DASHBOARD_SLOW_QUERIES_POLL_INTERVAL_MS = 15000;
const HISTORY_MAX_AGE_MS = 7 * 24 * 60 * 60 * 1000;
const HISTORY_RAW_AGE_MS = 60 * 60 * 1000;
const HISTORY_MINUTE_AGE_MS = 24 * 60 * 60 * 1000;
const HISTORY_MAX_SAMPLES = 5000;
const DASHBOARD_SLOW_QUERIES_LIMIT = 50;
const DASHBOARD_STATS_SQL = [
  "SELECT metric_name, metric_value",
  "FROM system.stats",
  "ORDER BY metric_name;",
].join("\n");

function parseInteger(value: string | undefined): number {
  if (!value) {
    return 0;
  }

  const parsed = Number.parseInt(value, 10);
  return Number.isNaN(parsed) ? 0 : parsed;
}

function parseNumber(value: string | undefined): number {
  if (!value) {
    return 0;
  }

  const parsed = Number.parseFloat(value);
  return Number.isFinite(parsed) ? parsed : 0;
}

function trimTrailingZeros(value: string): string {
  if (!value.includes(".")) {
    return value;
  }

  return value.replace(/\.0+$/, "").replace(/(\.\d*?)0+$/, "$1");
}

function formatCompactNumber(value: number, digits = 1): string {
  const abs = Math.abs(value);
  if (abs < 1000) {
    return value.toLocaleString();
  }

  const units = [
    { threshold: 1_000_000_000_000, suffix: "t" },
    { threshold: 1_000_000_000, suffix: "b" },
    { threshold: 1_000_000, suffix: "m" },
    { threshold: 1_000, suffix: "k" },
  ];

  for (const unit of units) {
    if (abs >= unit.threshold) {
      const scaled = value / unit.threshold;
      return `${trimTrailingZeros(scaled.toFixed(digits))}${unit.suffix}`;
    }
  }

  return value.toLocaleString();
}

function buildCountMetricItem(label: string, value: string | undefined): StatGroupItem {
  const numeric = parseInteger(value);
  const exactValue = numeric.toLocaleString();
  const displayValue = formatCompactNumber(numeric);
  return {
    label,
    value: displayValue,
    exactValue: displayValue === exactValue ? undefined : exactValue,
  };
}

function buildDecimalMetricItem(
  label: string,
  value: string | undefined,
  digits = 2,
  suffix = "",
): StatGroupItem {
  const numeric = parseNumber(value);
  const exactCore = numeric.toLocaleString(undefined, {
    minimumFractionDigits: digits,
    maximumFractionDigits: digits,
  });
  const exactValue = `${exactCore}${suffix}`;
  const compactCore =
    Math.abs(numeric) >= 1000 ? formatCompactNumber(numeric, digits > 1 ? 1 : digits) : exactCore;
  const displayValue = `${compactCore}${suffix}`;
  return {
    label,
    value: displayValue,
    exactValue: displayValue === exactValue ? undefined : exactValue,
  };
}

function compactDashboardSamples(samples: DashboardMetricSample[]): DashboardMetricSample[] {
  const now = Date.now();
  const buckets = new Map<string, DashboardMetricSample>();

  for (const sample of samples) {
    const age = now - sample.sampled_at;
    if (!Number.isFinite(sample.sampled_at) || age < 0 || age > HISTORY_MAX_AGE_MS) {
      continue;
    }

    const bucketSize = age <= HISTORY_RAW_AGE_MS ? 1000 : age <= HISTORY_MINUTE_AGE_MS ? 60_000 : 300_000;
    const bucket = Math.floor(sample.sampled_at / bucketSize);
    buckets.set(`${bucketSize}:${bucket}`, sample);
  }

  const compacted = Array.from(buckets.values()).sort((a, b) => a.sampled_at - b.sampled_at);
  return compacted.length > HISTORY_MAX_SAMPLES ? compacted.slice(-HISTORY_MAX_SAMPLES) : compacted;
}

function appendStatsSample(samples: DashboardMetricSample[], stats: SystemStatsMap): DashboardMetricSample[] {
  const sample = statsMapToDashboardSample(stats);
  if (!sample) {
    return samples;
  }

  const lastSample = samples.length > 0 ? samples[samples.length - 1] : undefined;
  const nextSamples = lastSample && sample.sampled_at - lastSample.sampled_at < 1000
    ? [...samples.slice(0, -1), sample]
    : [...samples, sample];

  return compactDashboardSamples(nextSamples);
}

function filterSamplesForRange(samples: DashboardMetricSample[], timeRange: string): DashboardMetricSample[] {
  const cutoff = getTimeRangeCutoff(timeRange);
  return cutoff === 0 ? samples : samples.filter((sample) => sample.sampled_at >= cutoff);
}

function formatStartTime(uptimeSeconds: string | undefined): string | undefined {
  const total = parseInteger(uptimeSeconds);
  if (total <= 0) {
    return undefined;
  }

  const started = new Date(Date.now() - total * 1000);
  const clock = started.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  if (started.toDateString() === new Date().toDateString()) {
    return `Running since ${clock}`;
  }

  const day = started.toLocaleDateString([], { month: "short", day: "numeric" });
  return `Running since ${day}, ${clock}`;
}

function formatUptime(seconds: string | undefined): string {
  const total = parseInteger(seconds);
  if (total <= 0) {
    return "-";
  }

  const days = Math.floor(total / 86400);
  const hours = Math.floor((total % 86400) / 3600);
  const minutes = Math.floor((total % 3600) / 60);

  if (days > 0) {
    return `${days}d ${hours}h ${minutes}m`;
  }
  if (hours > 0) {
    return `${hours}h ${minutes}m`;
  }

  return `${minutes}m`;
}

export default function Dashboard() {
  const { user } = useAuth();
  const navigate = useNavigate();
  const [timeRange, setTimeRange] = useState("24 HOURS");
  const [selectedStorageId, setSelectedStorageId] = useState("");
  const [metricSamples, setMetricSamples] = useState<DashboardMetricSample[]>([]);

  const {
    data: stats,
    isFetching: isLoading,
    error,
    refetch: refetchStats,
  } = useGetStatsQuery(undefined, {
    pollingInterval: DASHBOARD_STATS_POLL_INTERVAL_MS,
    refetchOnFocus: true,
    refetchOnReconnect: true,
  });
  const {
    data: storages = [],
    refetch: refetchStorages,
  } = useGetStoragesQuery();
  const {
    data: clusterSnapshot,
    isFetching: isClusterLoading,
    error: clusterError,
    refetch: refetchCluster,
  } = useGetClusterSnapshotQuery(undefined, {
    pollingInterval: 5000,
  });
  const {
    data: slowQueries = [],
    isFetching: isSlowQueriesLoading,
    refetch: refetchSlowQueries,
  } = useGetSlowQueriesQuery(DASHBOARD_SLOW_QUERIES_LIMIT, {
    pollingInterval: DASHBOARD_SLOW_QUERIES_POLL_INTERVAL_MS,
    refetchOnFocus: true,
    refetchOnReconnect: true,
  });
  const [checkStorageHealth, { data: storageHealth, isLoading: isStorageHealthLoading, error: storageHealthError }] =
    useCheckStorageHealthMutation();

  const currentStats = stats ?? EMPTY_STATS;
  const isInitialLoad = isLoading && !stats;
  const visibleMetricSamples = useMemo(
    () => filterSamplesForRange(metricSamples, timeRange),
    [metricSamples, timeRange],
  );

  useEffect(() => {
    if (!stats) {
      return;
    }

    setMetricSamples((samples) => appendStatsSample(samples, stats));
  }, [stats]);

  useEffect(() => {
    if (!selectedStorageId && storages.length > 0) {
      setSelectedStorageId(storages[0].storage_id);
    }
  }, [selectedStorageId, storages]);

  useEffect(() => {
    if (!selectedStorageId) {
      return;
    }

    void checkStorageHealth({ storageId: selectedStorageId, extended: true });
  }, [checkStorageHealth, selectedStorageId]);

  async function handleRefresh(): Promise<void> {
    await Promise.all([refetchStats(), refetchStorages(), refetchCluster(), refetchSlowQueries()]);

    if (selectedStorageId) {
      await checkStorageHealth({ storageId: selectedStorageId, extended: true });
    }
  }

  const clusterErrorMessage =
    clusterError && "error" in clusterError && typeof clusterError.error === "string"
      ? clusterError.error
      : clusterError
        ? "Failed to fetch cluster information"
        : null;

  const throughputSeries = useMemo(
    () => visibleMetricSamples.map((sample) => Number(sample.queries_per_second)),
    [visibleMetricSamples],
  );
  const latencySeries = useMemo(
    () => visibleMetricSamples.map((sample) => Number(sample.avg_query_latency_ms)),
    [visibleMetricSamples],
  );

  /**
   * Pinned to the snapshot that reported the uptime. Recomputing per render would walk the
   * start time forward whenever an unrelated poll rerendered the page.
   */
  const startedLabel = useMemo(
    () => formatStartTime(stats?.server_uptime_seconds),
    [stats],
  );

  const connections = parseInteger(currentStats.active_connections);
  const peakConnections = parseInteger(currentStats.active_connections_peak);
  const connectionPercent = peakConnections > 0 ? Math.min(100, (connections / peakConnections) * 100) : 0;

  const heroMetrics: HeroMetric[] = [
    {
      label: "Uptime",
      icon: Clock3,
      value: currentStats.server_uptime_human || formatUptime(currentStats.server_uptime_seconds),
      detail: startedLabel,
    },
    {
      label: "Avg Latency",
      icon: Timer,
      value: buildDecimalMetricItem("", currentStats.avg_query_latency_ms, 2, " ms").value,
      detail: "Mean SQL duration",
      delta: percentDelta(latencySeries),
      deltaGoodWhen: "down",
    },
    {
      label: "Throughput",
      icon: Gauge,
      value: `${buildDecimalMetricItem("", currentStats.queries_per_second).value} QPS`,
      detail: "Queries per second",
      delta: percentDelta(throughputSeries),
    },
    {
      label: "Active Connections",
      icon: Plug,
      value: connections.toLocaleString(),
      denominator: `/ ${peakConnections.toLocaleString()}`,
      progress: { percent: connectionPercent, label: `${Math.round(connectionPercent)}%` },
    },
  ];

  return (
    <section data-dashboard className="flex min-h-full flex-col gap-5 bg-[var(--kdb-page)] p-4 lg:p-6">
      <DashboardHero
        username={user?.username ?? "admin"}
        serverVersion={currentStats.server_version}
        metrics={heroMetrics}
        isMetricsLoading={isInitialLoad}
        actions={
          <div className="flex flex-wrap items-center gap-2">
            <Button
              variant="outline"
              size="sm"
              onClick={() => {
                navigate("/sql", {
                  state: {
                    prefillTitle: "Dashboard Stats",
                    prefillSql: DASHBOARD_STATS_SQL,
                  },
                });
              }}
            >
              <DatabaseZap className="mr-1.5 h-4 w-4" />
              Open Stats SQL
            </Button>
            <Select value={timeRange} onValueChange={setTimeRange}>
              <SelectTrigger className="h-9 w-[140px]">
                <SelectValue placeholder="Time range" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="1 HOURS">Last 1 Hour</SelectItem>
                <SelectItem value="6 HOURS">Last 6 Hours</SelectItem>
                <SelectItem value="24 HOURS">Last 24 Hours</SelectItem>
                <SelectItem value="7 DAYS">Last 7 Days</SelectItem>
              </SelectContent>
            </Select>
            <Button
              variant="outline"
              size="sm"
              onClick={() => void handleRefresh()}
              disabled={isLoading || isStorageHealthLoading}
            >
              <RefreshCw className={`mr-1.5 h-4 w-4 ${isLoading || isStorageHealthLoading ? "animate-spin" : ""}`} />
              Refresh
            </Button>
          </div>
        }
      />

      <div className="grid gap-4 xl:grid-cols-[minmax(0,1.55fr)_minmax(330px,1fr)]">
        <QueryActivityChart
          data={visibleMetricSamples}
          isLoading={isInitialLoad}
          className="h-full"
        />

        <div className="flex flex-col gap-4">
          <StatGroupPanel
            index={1}
            icon={Database}
            title="Database Overview"
            isLoading={isInitialLoad}
            items={[
              buildCountMetricItem("Tables", currentStats.total_tables),
              buildCountMetricItem("Schemas", currentStats.total_namespaces),
              buildCountMetricItem("SQL Statements", currentStats.queries_total),
            ]}
          />
          <StatGroupPanel
            index={2}
            icon={Wifi}
            title="Connections & Subscriptions"
            isLoading={isInitialLoad}
            action={
              <span className={ACCENT_CHIP}>
                <span className="tabular-nums">
                  {buildDecimalMetricItem("", currentStats.subscription_changes_delivered_per_second).value}
                </span>
                changes/s
              </span>
            }
            items={[
              buildCountMetricItem("Connections", currentStats.active_connections),
              buildCountMetricItem("Peak Connections", currentStats.active_connections_peak),
              buildCountMetricItem("Subscriptions", currentStats.active_subscriptions),
              buildCountMetricItem("Peak Subscriptions", currentStats.active_subscriptions_peak),
            ]}
          />
          <StatGroupPanel
            index={3}
            icon={MessageSquare}
            title="Pub/Sub"
            isLoading={isInitialLoad}
            items={[
              buildCountMetricItem("Messages", currentStats.pubsub_messages_published_total),
              buildCountMetricItem("Consumer Groups", currentStats.topic_consumer_group_count),
              buildCountMetricItem("Active Consumers", currentStats.pubsub_active_consumers),
              buildCountMetricItem("Topics", currentStats.topic_cache_topic_count),
            ]}
          />
        </div>
      </div>

      <div className="grid gap-4 xl:grid-cols-3">
        <ResourcePanel index={0} samples={visibleMetricSamples} isLoading={isInitialLoad} />
        <StorageUsageChart
          index={1}
          storages={storages}
          selectedStorageId={selectedStorageId}
          onStorageChange={setSelectedStorageId}
          health={storageHealth ?? null}
          isLoading={isStorageHealthLoading}
          error={storageHealthError && "error" in storageHealthError ? storageHealthError.error : null}
        />
        <DashboardClusterOverview
          index={2}
          health={clusterSnapshot?.health ?? null}
          nodes={clusterSnapshot?.nodes ?? []}
          isLoading={isClusterLoading}
          error={clusterErrorMessage}
        />
      </div>

      {error && (
        <Card className="border-destructive/30 bg-destructive/5 px-5 py-4 text-sm text-destructive">
          {"error" in error ? error.error : "Failed to fetch dashboard stats"}
        </Card>
      )}

      <div className="mt-2 flex items-center gap-4">
        <span className={PANEL_LABEL}>Detailed metrics</span>
        <span className="h-px flex-1 bg-border" />
      </div>

      <MetricsChart
        data={visibleMetricSamples}
        isLoading={isLoading && visibleMetricSamples.length === 0}
        trailingPanel={
          <SlowQueriesPanel
            queries={slowQueries}
            isLoading={isSlowQueriesLoading && slowQueries.length === 0}
          />
        }
      />
    </section>
  );
}
