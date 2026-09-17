import { describe, expect, it, vi } from "vitest";

vi.mock("./kalam-client", () => ({
  executeQuery: vi.fn(),
  getCurrentToken: vi.fn(),
}));
vi.mock("./backend-url", () => ({
  getApiBaseUrl: () => "http://127.0.0.1:2900",
}));

import { sqlResponseFromQuery } from "./api";

const schema = [
  { name: "schedule_id", data_type: "Text" as const, index: 0 },
  { name: "run_count", data_type: "BigInt" as const, index: 1 },
];

describe("sqlResponseFromQuery", () => {
  it("maps named_rows when positional rows are omitted", () => {
    const result = sqlResponseFromQuery({
      status: "success",
      took: 3,
      results: [
        {
          schema,
          row_count: 1,
          named_rows: [{ schedule_id: "reports.daily", run_count: 4 }],
        },
      ],
    });

    expect(result.row_count).toBe(1);
    expect(result.rows).toHaveLength(1);
    expect(result.rows[0][0].toJson()).toBe("reports.daily");
    expect(result.rows[0][1].toJson()).toBe(4);
  });

  it("throws on query errors instead of returning an empty table", () => {
    expect(() =>
      sqlResponseFromQuery({
        status: "error",
        error: { code: "SQL_EXECUTION_ERROR", message: "Permission denied" },
      }),
    ).toThrow("Permission denied");
  });
});
