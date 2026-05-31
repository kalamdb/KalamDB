import { useEffect, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import {
  Clock3,
  DatabaseZap,
  Database,
  Gauge,
  MessageSquare,
  RefreshCw,
  Search,
  Timer,
  Wifi,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { DashboardClusterOverview } from "@/components/dashboard/ClusterOverview";
import { MetricsChart } from "@/components/dashboard/MetricsChart";
import { SlowQueriesPanel } from "@/components/dashboard/SlowQueriesPanel";
import { StorageUsageChart } from "@/components/dashboard/StorageUsageChart";
import { PageLayout } from "@/components/layout/PageLayout";
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

interface MetricCardItem {
  label: string;
  displayValue: string;
  exactValue?: string;
}

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

function buildCountMetricItem(label: string, value: string | undefined): MetricCardItem {
  const numeric = parseInteger(value);
  const exactValue = numeric.toLocaleString();
  const displayValue = formatCompactNumber(numeric);
  return {
    label,
    displayValue,
    exactValue: displayValue === exactValue ? undefined : exactValue,
  };
}

function buildDecimalMetricItem(
  label: string,
  value: string | undefined,
  digits = 2,
  suffix = "",
): MetricCardItem {
  const numeric = parseNumber(value);
  const exactCore = numeric.toLocaleString(undefined, {
    minimumFractionDigits: digits,
    maximumFractionDigits: digits,
  });
  const exactValue = `${exactCore}${suffix}`;
  const compactCore = Math.abs(numeric) >= 1000 ? formatCompactNumber(numeric, digits > 1 ? 1 : digits) : exactCore;
  const displayValue = `${compactCore}${suffix}`;
  return {
    label,
    displayValue,
    exactValue: displayValue === exactValue ? undefined : exactValue,
  };
}

function cardItemsClass(itemCount: number): string {
  if (itemCount === 1) {
    return "flex justify-center";
  }
  if (itemCount === 3) {
    return "grid grid-cols-3 gap-2";
  }
  return "grid grid-cols-2 gap-3";
}

function cardValueClass(itemCount: number): string {
  return itemCount > 2
    ? "max-w-full break-words text-center text-xl font-semibold leading-tight tabular-nums"
    : "max-w-full break-words text-center text-2xl font-semibold leading-tight tabular-nums";
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

  const cards = [
    {
      title: "Uptime",
      items: [
        {
          label: "Process",
          displayValue: currentStats.server_uptime_human || formatUptime(currentStats.server_uptime_seconds),
        },
      ],
      icon: Clock3,
    },
    {
      title: "Tables & Namespaces",
      items: [
        buildCountMetricItem("Tables", currentStats.total_tables),
        buildCountMetricItem("Namespaces", currentStats.total_namespaces),
      ],
      icon: Database,
    },
    {
      title: "Connections & Subscriptions",
      items: [
        buildCountMetricItem("Connections", currentStats.active_connections),
        buildCountMetricItem("Peak Connections", currentStats.active_connections_peak),
        buildCountMetricItem("Subscriptions", currentStats.active_subscriptions),
        buildCountMetricItem("Peak Subscriptions", currentStats.active_subscriptions_peak),
        buildDecimalMetricItem("Changes/s", currentStats.subscription_changes_delivered_per_second),
      ],
      icon: Wifi,
    },
    {
      title: "Pub/Sub",
      items: [
        buildCountMetricItem("Messages", currentStats.pubsub_messages_published_total),
        buildCountMetricItem("Consumer Groups", currentStats.topic_consumer_group_count),
        buildCountMetricItem("Active Consumers", currentStats.pubsub_active_consumers),
        buildCountMetricItem("Topics", currentStats.topic_cache_topic_count),
      ],
      icon: MessageSquare,
    },
    {
      title: "Queries Total",
      items: [buildCountMetricItem("SQL statements", currentStats.queries_total)],
      icon: Search,
    },
    {
      title: "Queries/s",
      items: [buildDecimalMetricItem("Throughput", currentStats.queries_per_second)],
      icon: Gauge,
    },
    {
      title: "Avg Latency",
      items: [buildDecimalMetricItem("Mean SQL duration", currentStats.avg_query_latency_ms, 2, " ms")],
      icon: Timer,
    },
  ];

  return (
    <PageLayout
      title="Dashboard"
      description={`Welcome back, ${user?.username ?? "admin"}`}
      actions={
        <div className="flex items-center gap-3">
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
    >
      <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-4 2xl:grid-cols-7">
        {cards.map((card) => (
          <Card key={card.title} className="min-h-[142px]">
            <CardContent className="flex flex-1 flex-col justify-center text-center">
              <div className="relative mb-3 flex min-h-8 items-start justify-center px-5">
                <p className="text-center text-xs uppercase leading-snug tracking-[0.14em] text-muted-foreground">
                  {card.title}
                </p>
                <card.icon className="absolute right-0 top-0 h-4 w-4 text-muted-foreground" />
              </div>
              <div className={cardItemsClass(card.items.length)}>
                {card.items.map((item) => (
                  <div key={item.label} className="flex min-w-0 flex-col items-center text-center">
                    <TooltipProvider delayDuration={200}>
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <p className={cardValueClass(card.items.length)}>{item.displayValue}</p>
                        </TooltipTrigger>
                        {item.exactValue ? <TooltipContent>{item.exactValue}</TooltipContent> : null}
                      </Tooltip>
                    </TooltipProvider>
                    <p className="text-center text-xs leading-snug text-muted-foreground">{item.label}</p>
                  </div>
                ))}
              </div>
            </CardContent>
          </Card>
        ))}
      </div>

      <MetricsChart
        data={visibleMetricSamples}
        isLoading={isLoading && visibleMetricSamples.length === 0}
        trailingPanel={<SlowQueriesPanel queries={slowQueries} isLoading={isSlowQueriesLoading && slowQueries.length === 0} />}
      />

      <div className="mt-6 grid gap-6 xl:grid-cols-[minmax(0,1.35fr)_minmax(340px,0.95fr)]">
        <StorageUsageChart
          storages={storages}
          selectedStorageId={selectedStorageId}
          onStorageChange={setSelectedStorageId}
          health={storageHealth ?? null}
          isLoading={isStorageHealthLoading}
          error={storageHealthError && "error" in storageHealthError ? storageHealthError.error : null}
        />

        <DashboardClusterOverview
          health={clusterSnapshot?.health ?? null}
          nodes={clusterSnapshot?.nodes ?? []}
          isLoading={isClusterLoading}
          error={clusterErrorMessage}
        />
      </div>

      {error && (
        <Card className="border-destructive/30 bg-destructive/5">
          <CardContent className="pt-4 text-sm text-destructive">
            {"error" in error ? error.error : "Failed to fetch dashboard stats"}
          </CardContent>
        </Card>
      )}
    </PageLayout>
  );
}
