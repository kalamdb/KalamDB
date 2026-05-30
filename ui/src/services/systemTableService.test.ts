import { describe, expect, it, vi } from "vitest";
import { fetchSystemStats } from "@/services/systemTableService";

const mockGetDb = vi.fn();

vi.mock("@/lib/db", () => ({
  getDb: () => mockGetDb(),
}));

vi.mock("@/lib/schema", () => ({
  system_settings: Symbol("system_settings"),
  system_slow_queries: Symbol("system_slow_queries"),
  system_stats: Symbol("system_stats"),
}));

describe("fetchSystemStats", () => {
  it("requests up to 200 system.stats rows for the dashboard metrics query", async () => {
    const rows = Array.from({ length: 200 }, (_, index) => ({
      metric_name: `metric_${index + 1}`,
      metric_value: index + 1,
    }));

    const limit = vi.fn().mockResolvedValue(rows);

    mockGetDb.mockReturnValue({
      select: vi.fn().mockReturnValue({
        from: vi.fn().mockReturnValue({
          limit,
        }),
      }),
    });

    const stats = await fetchSystemStats();

    expect(limit).toHaveBeenCalledWith(200);
    expect(Object.keys(stats)).toHaveLength(200);
    expect(stats.metric_200).toBe("200");
  });
});