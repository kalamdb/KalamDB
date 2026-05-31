import { describe, expect, it } from "vitest";
import { buildMetricChartData } from "@/components/dashboard/MetricsChart";

describe("buildMetricChartData", () => {
  it("preserves backend-provided rate metrics in timestamp order", () => {
    const chartData = buildMetricChartData([
      {
        sampled_at: 6_000,
        select_queries_per_second: 2,
        insert_queries_per_second: 1,
        update_queries_per_second: 0.5,
        delete_queries_per_second: 0.25,
        manifest_writes_per_second: 1.5,
        manifest_reads_per_second: 1.25,
        parquet_files_written_per_second: 2.5,
        parquet_files_read_per_second: 1.75,
        manifest_cache_rocksdb_entries: 9,
        active_connections_peak: 6,
        subscription_changes_delivered_per_second: 12,
        subscription_bytes_delivered_per_second: 2048,
        pubsub_active_consumers: 4,
        pubsub_messages_consumed_per_second: 3,
        pubsub_messages_consumed_peak_per_second: 9,
        pubsub_kb_consumed_per_second: 2.5,
        topic_cache_topic_count: 8,
      },
      {
        sampled_at: 1_000,
        select_queries_per_second: 1,
        insert_queries_per_second: 0.5,
        update_queries_per_second: 0.25,
        delete_queries_per_second: 0,
        manifest_writes_per_second: 0.75,
        manifest_reads_per_second: 0.5,
        parquet_files_written_per_second: 1,
        parquet_files_read_per_second: 0.5,
        manifest_cache_rocksdb_entries: 7,
        active_connections_peak: 5,
        subscription_changes_delivered_per_second: 2,
        subscription_bytes_delivered_per_second: 256,
        pubsub_active_consumers: 1,
        pubsub_messages_consumed_per_second: 0.5,
        pubsub_messages_consumed_peak_per_second: 4,
        pubsub_kb_consumed_per_second: 0.25,
        topic_cache_topic_count: 3,
      },
    ]);

    expect(chartData).toHaveLength(2);
    expect(chartData[0].timestamp).toBe(1_000);
    expect(chartData[1].timestamp).toBe(6_000);
    expect(chartData[0].select_queries_per_second).toBe(1);
    expect(chartData[1].select_queries_per_second).toBe(2);
    expect(chartData[1].insert_queries_per_second).toBe(1);
    expect(chartData[1].update_queries_per_second).toBe(0.5);
    expect(chartData[1].delete_queries_per_second).toBe(0.25);
    expect(chartData[1].manifest_writes_per_second).toBe(1.5);
    expect(chartData[1].manifest_reads_per_second).toBe(1.25);
    expect(chartData[1].parquet_files_written_per_second).toBe(2.5);
    expect(chartData[1].parquet_files_read_per_second).toBe(1.75);
    expect(chartData[1].manifest_cache_rocksdb_entries).toBe(9);
    expect(chartData[1].active_connections_peak).toBe(6);
    expect(chartData[1].subscription_changes_delivered_per_second).toBe(12);
    expect(chartData[1].subscription_bytes_delivered_per_second).toBe(2048);
    expect(chartData[1].pubsub_active_consumers).toBe(4);
    expect(chartData[1].pubsub_messages_consumed_per_second).toBe(3);
    expect(chartData[1].pubsub_messages_consumed_peak_per_second).toBe(9);
    expect(chartData[1].pubsub_kb_consumed_per_second).toBe(2.5);
    expect(chartData[1].topic_cache_topic_count).toBe(8);
  });

  it("skips samples with invalid timestamps", () => {
    const chartData = buildMetricChartData([
      {
        sampled_at: Number.NaN,
        select_queries_per_second: 8,
      },
      {
        sampled_at: 0,
        manifest_reads_per_second: 1,
      },
      {
        sampled_at: 6_000,
        select_queries_per_second: 1,
      },
    ]);

    expect(chartData).toHaveLength(1);
    expect(chartData[0].select_queries_per_second).toBe(1);
  });
});