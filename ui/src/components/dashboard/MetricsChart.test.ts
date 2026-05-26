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