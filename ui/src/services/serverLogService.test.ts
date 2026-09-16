import { describe, expect, it, vi } from "vitest";
import { fetchServerLogs } from "@/services/serverLogService";

const mockGetDb = vi.fn();

vi.mock("@/lib/db", () => ({
  getDb: () => mockGetDb(),
  sqlWithInlineParams: (sql: string) => sql,
}));

vi.mock("@kalamdb/orm", () => ({
  compileQuery: (query: { toSQL?: () => { sql: string; params: unknown[] } }) =>
    query.toSQL?.() ?? { sql: "", params: [] },
}));

vi.mock("drizzle-orm", () => ({
  eq: vi.fn((column, value) => ({ column, value })),
  like: vi.fn((column, value) => ({ column, value })),
  desc: vi.fn((column) => column),
  and: vi.fn((...conditions) => conditions),
  lt: vi.fn((column, value) => ({ column, value })),
  gt: vi.fn((column, value) => ({ column, value })),
  sql: vi.fn(),
}));

vi.mock("@/lib/schema", () => ({
  system_server_logs: {
    timestamp: "timestamp",
    level: "level",
    thread: "thread",
    target: "target",
    line: "line",
    message: "message",
  },
}));

describe("fetchServerLogs", () => {
  it("queries system.server_logs through the ORM with newest-first limit", async () => {
    const rows = [{ timestamp: "2026-09-16T00:00:00Z", level: "INFO", message: "booted" }];
    const limit = vi.fn().mockResolvedValue(rows);
    const orderBy = vi.fn().mockReturnValue({ limit });
    const where = vi.fn().mockReturnValue({ orderBy });
    const from = vi.fn().mockReturnValue({ where });
    const select = vi.fn().mockReturnValue({ from });

    mockGetDb.mockReturnValue({ select });

    const data = await fetchServerLogs({ limit: 50, level: "INFO" });

    expect(select).toHaveBeenCalledTimes(1);
    expect(from).toHaveBeenCalledTimes(1);
    expect(where).toHaveBeenCalledTimes(1);
    expect(orderBy).toHaveBeenCalledTimes(1);
    expect(limit).toHaveBeenCalledWith(50);
    expect(data).toEqual(rows);
  });
});
