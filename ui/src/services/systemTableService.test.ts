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
  it("returns all available system.stats rows without truncating to 100", async () => {
    const rows = Array.from({ length: 150 }, (_, index) => ({
      metric_name: `metric_${index + 1}`,
      metric_value: index + 1,
    }));

    const limit = vi.fn().mockResolvedValue(rows.slice(0, 100));
    const from = vi.fn().mockImplementation(async (resolve: (value: typeof rows) => unknown) => resolve(rows));

    mockGetDb.mockReturnValue({
      select: vi.fn().mockReturnValue({
        from: vi.fn().mockReturnValue({
          then: from,
          limit,
        }),
      }),
    });

    const stats = await fetchSystemStats();

    expect(Object.keys(stats)).toHaveLength(150);
    expect(stats.metric_150).toBe("150");
    expect(limit).not.toHaveBeenCalled();
  });
});