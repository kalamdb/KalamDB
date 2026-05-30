// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import Dashboard from "@/pages/Dashboard";

const mockUseAuth = vi.fn();
const mockGetStatsQuery = vi.fn();
const mockGetStoragesQuery = vi.fn();
const mockGetClusterSnapshotQuery = vi.fn();
const mockCheckStorageHealth = vi.fn();
const mockCheckStorageHealthMutation = vi.fn();

vi.mock("@/lib/auth", () => ({
  useAuth: () => mockUseAuth(),
}));

vi.mock("@/store/apiSlice", () => ({
  useGetStatsQuery: () => mockGetStatsQuery(),
  useGetStoragesQuery: () => mockGetStoragesQuery(),
  useGetClusterSnapshotQuery: () => mockGetClusterSnapshotQuery(),
  useCheckStorageHealthMutation: () => mockCheckStorageHealthMutation(),
}));

vi.mock("@/components/dashboard/MetricsChart", () => ({
  MetricsChart: ({ data }: { data: unknown[] }) => <div>Metrics chart {data.length}</div>,
}));

vi.mock("@/components/dashboard/StorageUsageChart", () => ({
  StorageUsageChart: ({ selectedStorageId }: { selectedStorageId: string }) => (
    <div>Storage usage {selectedStorageId}</div>
  ),
}));

vi.mock("@/components/dashboard/ClusterOverview", () => ({
  DashboardClusterOverview: ({ nodes }: { nodes: unknown[] }) => (
    <div>Cluster overview {nodes.length}</div>
  ),
}));

describe("Dashboard page", () => {
  beforeEach(() => {
    cleanup();
    vi.stubGlobal(
      "ResizeObserver",
      class ResizeObserver {
        observe() {}
        unobserve() {}
        disconnect() {}
      },
    );

    mockUseAuth.mockReset();
    mockGetStatsQuery.mockReset();
    mockGetStoragesQuery.mockReset();
    mockGetClusterSnapshotQuery.mockReset();
    mockCheckStorageHealth.mockReset();
    mockCheckStorageHealthMutation.mockReset();

    mockUseAuth.mockReturnValue({
      user: { username: "root", role: "system" },
    });

    mockGetStatsQuery.mockReturnValue({
      data: {
        server_version: "v1.2.3",
        total_namespaces: "9",
        total_tables: "42",
        active_connections: "3",
        active_subscriptions: "2",
        active_subscriptions_peak: "7",
        websocket_sessions_peak: "5",
        server_uptime_human: "1h 10m",
        queries_total: "12",
        queries_per_second: "1.25",
        avg_query_latency_ms: "4.50",
        select_queries_total: "2",
        insert_queries_total: "6",
        update_queries_total: "3",
        delete_queries_total: "1",
        memory_usage_mb: "64",
        cpu_usage_percent: "8.5",
        open_files_total: "20",
        open_files_directories: "4",
        manifest_cache_rocksdb_entries: "11",
        parquet_files_written_total: "8",
        parquet_files_read_total: "13",
      },
      isFetching: false,
      error: null,
      refetch: vi.fn().mockResolvedValue(undefined),
    });

    mockGetStoragesQuery.mockReturnValue({
      data: [{ storage_id: "local", name: "Local" }],
      refetch: vi.fn().mockResolvedValue(undefined),
    });

    mockGetClusterSnapshotQuery.mockReturnValue({
      data: {
        health: {
          healthy: true,
          totalNodes: 1,
          activeNodes: 1,
          offlineNodes: 0,
          leaderNodes: 1,
          followerNodes: 0,
          joiningNodes: 0,
          catchingUpNodes: 0,
        },
        nodes: [
          {
            cluster_id: "local",
            node_id: 1,
            role: "leader",
            status: "active",
            rpc_addr: "127.0.0.1:9080",
            api_addr: "127.0.0.1:2900",
            is_self: true,
            is_leader: true,
            groups_leading: 1,
            total_groups: 1,
            current_term: 1,
            last_applied_log: 10,
            leader_last_log_index: 10,
            snapshot_index: 0,
            catchup_progress_pct: null,
            replication_lag: 0,
            hostname: "localhost",
            version: "v1.2.3",
            memory_mb: 42,
            os: "macOS",
            arch: "arm64",
          },
        ],
      },
      isFetching: false,
      error: null,
      refetch: vi.fn().mockResolvedValue(undefined),
    });

    mockCheckStorageHealth.mockResolvedValue(undefined);
    mockCheckStorageHealthMutation.mockReturnValue([
      mockCheckStorageHealth,
      {
        data: { storage_id: "local", healthy: true },
        isLoading: false,
        error: null,
      },
    ]);
  });

  afterEach(() => {
    cleanup();
    vi.clearAllMocks();
    vi.unstubAllGlobals();
  });

  it("renders dashboard metrics from the admin stats queries", async () => {
    render(<Dashboard />);

    expect(screen.getByText("Dashboard")).toBeTruthy();
    expect(screen.getByText("Welcome back, root")).toBeTruthy();
    expect(screen.getByText("1h 10m")).toBeTruthy();
    expect(screen.getByText("42")).toBeTruthy();
    expect(screen.getByText("9")).toBeTruthy();
    expect(screen.getByText("3")).toBeTruthy();
    expect(screen.getByText("2")).toBeTruthy();
    expect(screen.getByText("12")).toBeTruthy();
    expect(screen.getByText("1.25")).toBeTruthy();
    expect(screen.getByText("4.50 ms")).toBeTruthy();
    expect(screen.getByText("Cluster overview 1")).toBeTruthy();

    await waitFor(() => {
      expect(screen.getByText("Metrics chart 1")).toBeTruthy();
    });

    await waitFor(() => {
      expect(mockCheckStorageHealth).toHaveBeenCalledWith({ storageId: "local", extended: true });
    });
  });

  it("renders metric charts above storage and cluster health", async () => {
    render(<Dashboard />);

    const metricsChart = await screen.findByText("Metrics chart 1");
    const storageWidget = screen.getByText("Storage usage local");

    const metricsPosition = metricsChart.compareDocumentPosition(storageWidget);
    expect(metricsPosition & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("refreshes dashboard queries and rechecks storage health", async () => {
    const statsRefetch = vi.fn().mockResolvedValue(undefined);
    const storagesRefetch = vi.fn().mockResolvedValue(undefined);
    const clusterRefetch = vi.fn().mockResolvedValue(undefined);

    mockGetStatsQuery.mockReturnValue({
      data: {
        server_version: "v1.2.3",
        total_namespaces: "9",
        total_tables: "42",
        active_connections: "3",
        active_subscriptions: "2",
        active_subscriptions_peak: "7",
        websocket_sessions_peak: "5",
        server_uptime_human: "1h 10m",
        queries_total: "12",
        queries_per_second: "1.25",
        avg_query_latency_ms: "4.50",
        select_queries_total: "2",
        insert_queries_total: "6",
        update_queries_total: "3",
        delete_queries_total: "1",
        memory_usage_mb: "64",
        cpu_usage_percent: "8.5",
        open_files_total: "20",
        open_files_directories: "4",
        manifest_cache_rocksdb_entries: "11",
        parquet_files_written_total: "8",
        parquet_files_read_total: "13",
      },
      isFetching: false,
      error: null,
      refetch: statsRefetch,
    });
    mockGetStoragesQuery.mockReturnValue({
      data: [{ storage_id: "local", name: "Local" }],
      refetch: storagesRefetch,
    });
    mockGetClusterSnapshotQuery.mockReturnValue({
      data: {
        health: {
          healthy: true,
          totalNodes: 1,
          activeNodes: 1,
          offlineNodes: 0,
          leaderNodes: 1,
          followerNodes: 0,
          joiningNodes: 0,
          catchingUpNodes: 0,
        },
        nodes: [
          {
            cluster_id: "local",
            node_id: 1,
            role: "leader",
            status: "active",
            rpc_addr: "127.0.0.1:9080",
            api_addr: "127.0.0.1:2900",
            is_self: true,
            is_leader: true,
            groups_leading: 1,
            total_groups: 1,
            current_term: 1,
            last_applied_log: 10,
            leader_last_log_index: 10,
            snapshot_index: 0,
            catchup_progress_pct: null,
            replication_lag: 0,
            hostname: "localhost",
            version: "v1.2.3",
            memory_mb: 42,
            os: "macOS",
            arch: "arm64",
          },
        ],
      },
      isFetching: false,
      error: null,
      refetch: clusterRefetch,
    });

    render(<Dashboard />);

    await waitFor(() => {
      expect(mockCheckStorageHealth).toHaveBeenCalledTimes(1);
    });

    fireEvent.click(screen.getByRole("button", { name: /refresh/i }));

    await waitFor(() => {
      expect(statsRefetch).toHaveBeenCalledTimes(1);
      expect(storagesRefetch).toHaveBeenCalledTimes(1);
      expect(clusterRefetch).toHaveBeenCalledTimes(1);
      expect(mockCheckStorageHealth).toHaveBeenCalledTimes(2);
    });
  });
});
