import { beforeEach, describe, expect, it, vi } from "vitest";

const mockExecuteSql = vi.fn();
const mockSubscribe = vi.fn();
const mockDb = {
  select: vi.fn(),
  insert: vi.fn(),
  update: vi.fn(),
};

vi.mock("@/lib/kalam-client", () => ({
  executeSql: (...args: unknown[]) => mockExecuteSql(...args),
  subscribe: (...args: unknown[]) => mockSubscribe(...args),
}));

vi.mock("@/lib/db", () => ({
  getDb: () => mockDb,
}));

vi.mock("@kalamdb/orm", async () => {
  const { pgTable } = await import("drizzle-orm/pg-core");
  return {
    kTable: pgTable,
  };
});

describe("sqlStudioWorkspaceSyncService", () => {
  beforeEach(() => {
    vi.resetModules();
    mockExecuteSql.mockReset();
    mockSubscribe.mockReset();
    mockDb.select.mockReset();
    mockDb.insert.mockReset();
    mockDb.update.mockReset();
  });

  function mockSelectOnce(rows: unknown[]): void {
    const limit = vi.fn().mockResolvedValue(rows);
    const where = vi.fn().mockReturnValue({ limit });
    mockDb.select.mockReturnValueOnce({ from: vi.fn().mockReturnValue({ where }) });
  }

  it("hydrates legacy favorites before opening the live subscription", async () => {
    const workspace = {
      version: 1,
      tabs: [
        {
          id: "tab-1",
          name: "Favorite Query",
          query: "SELECT * FROM default.events",
          settings: {
            isDirty: false,
            isLive: false,
            liveStatus: "idle",
            resultView: "results",
            lastSavedAt: null,
            savedQueryId: "saved-1",
          },
        },
      ],
      savedQueries: [
        {
          id: "saved-1",
          title: "Favorite Query",
          sql: "SELECT * FROM default.events",
          lastSavedAt: "2026-04-23T00:00:00.000Z",
          isLive: false,
          openedRecently: true,
          isCurrentTab: true,
        },
      ],
      activeTabId: "tab-1",
      updatedAt: "2026-04-23T00:00:00.000Z",
    };

    const insertValues = vi.fn().mockResolvedValue(undefined);

    mockExecuteSql
      .mockResolvedValueOnce([]);
    mockSelectOnce([]);
    mockSelectOnce([{ id: "sql-studio-workspace", payload: workspace }]);
    mockSelectOnce([]);
    mockDb.insert.mockReturnValue({ values: insertValues });
    mockSubscribe.mockResolvedValue(async () => {});

    const { subscribeToSyncedSqlStudioWorkspaceState } = await import("@/services/sqlStudioWorkspaceSyncService");
    const onChange = vi.fn();

    await subscribeToSyncedSqlStudioWorkspaceState("admin", onChange);

    expect(onChange).toHaveBeenCalledWith(workspace);
    expect(mockSubscribe).toHaveBeenCalledWith(
      "SELECT * FROM dba.favorites WHERE id = 'sql-studio-state:admin:workspace'",
      expect.any(Function),
      { last_rows: 0, batch_size: 1, auto_fetch_batches: false },
    );
    expect(mockExecuteSql).toHaveBeenCalledWith(
      "CREATE TABLE IF NOT EXISTS dba.favorites (id TEXT PRIMARY KEY, payload JSON) WITH (TYPE='USER')",
    );
    expect(mockDb.insert).toHaveBeenCalled();
    expect(insertValues).toHaveBeenCalledWith(
      expect.objectContaining({
        id: "sql-studio-state:admin:workspace",
        payload: workspace,
      }),
    );
  });
});