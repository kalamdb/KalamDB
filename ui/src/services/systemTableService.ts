import { getDb } from "@/lib/db";
import type { SystemSettingRow, SystemSlowQueryRow } from "@/lib/models";
import { system_settings, system_slow_queries, system_stats } from "@/lib/schema";
import { desc } from "drizzle-orm";

export type Setting = SystemSettingRow;
export type SystemStatsMap = Record<string, string>;

export interface DashboardMetricSample {
  sampled_at: number;
  [metricName: string]: number;
}

export type SlowQuery = SystemSlowQueryRow;

export async function fetchSystemSettings(): Promise<Setting[]> {
  const db = getDb();
  return db.select().from(system_settings);
}

export function mapSettingsRows(rows: Setting[]): Setting[] {
  if (rows.length === 0) {
    const fallbackRows: Setting[] = [
      { name: "server.version", value: "0.1.0", description: "KalamDB server version", category: "server" },
      { name: "storage.default_backend", value: "rocksdb", description: "Default storage backend for write operations", category: "storage" },
      { name: "query.max_rows", value: "10000", description: "Maximum rows returned per query", category: "query" },
      { name: "auth.jwt_expiry", value: "3600", description: "JWT token expiry in seconds", category: "auth" },
    ];
    return fallbackRows;
  }
  return rows;
}

export async function fetchSystemStats(): Promise<SystemStatsMap> {
  const db = getDb();
  const rows = await db.select().from(system_stats).limit(200);
  const stats: SystemStatsMap = {};
  for (const row of rows) {
    if (row.metric_name) {
      stats[row.metric_name] = String(row.metric_value ?? "");
    }
  }
  return stats;
}

export const DASHBOARD_METRIC_KEYS = [
  "active_connections",
  "active_connections_peak",
  "active_subscriptions",
  "active_subscriptions_peak",
  "subscription_changes_delivered_per_second",
  "subscription_bytes_delivered_per_second",
  "pubsub_active_consumers",
  "pubsub_messages_consumed_per_second",
  "pubsub_messages_consumed_peak_per_second",
  "pubsub_kb_consumed_per_second",
  "topic_cache_topic_count",
  "memory_usage_mb",
  "cpu_usage_percent",
  "select_queries_per_second",
  "insert_queries_per_second",
  "update_queries_per_second",
  "delete_queries_per_second",
  "open_files_total",
  "open_files_directories",
  "manifest_cache_rocksdb_entries",
  "flush_operations_total",
  "manifest_reads_per_second",
  "manifest_writes_per_second",
  "parquet_files_written_per_second",
  "parquet_files_read_per_second",
];

function normalizeMetricValue(value: unknown): number {
  if (typeof value === "number") return Number.isFinite(value) ? value : Number.NaN;
  if (typeof value === "string") {
    const numeric = Number(value.trim());
    return Number.isFinite(numeric) ? numeric : Number.NaN;
  }
  return Number.NaN;
}

export function getTimeRangeCutoff(timeRange: string): number {
  const match = timeRange.trim().match(/^(\d+)\s+(HOUR|HOURS|DAY|DAYS)$/i);
  if (!match) return 0;
  const amount = Number(match[1]);
  if (!Number.isFinite(amount) || amount <= 0) return 0;
  const unit = match[2].toUpperCase();
  const multiplier = unit.startsWith("DAY") ? 24 * 60 * 60 * 1000 : 60 * 60 * 1000;
  return Date.now() - amount * multiplier;
}

export function statsMapToDashboardSample(stats: SystemStatsMap, sampledAt = Date.now()): DashboardMetricSample | null {
  const sample: DashboardMetricSample = { sampled_at: sampledAt };
  let hasMetric = false;

  for (const key of DASHBOARD_METRIC_KEYS) {
    const metricValue = normalizeMetricValue(stats[key]);
    if (Number.isFinite(metricValue)) {
      sample[key] = metricValue;
      hasMetric = true;
    }
  }

  return hasMetric ? sample : null;
}

export async function fetchSlowQueries(limit = 20): Promise<SlowQuery[]> {
  const db = getDb();
  return db
    .select()
    .from(system_slow_queries)
    .orderBy(desc(system_slow_queries.timestamp_ms))
    .limit(limit);
}
