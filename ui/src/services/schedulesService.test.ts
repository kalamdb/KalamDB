import { describe, expect, it, vi } from "vitest";
vi.mock("@/lib/api", () => ({ executeSql: vi.fn() }));
import { scheduleSqlName, scheduleStatus } from "./schedulesService";

describe("schedule monitoring", () => {
  it("quotes namespace and name independently", () => {
    expect(scheduleSqlName({ namespace_id: 'reports', name: 'daily"; DROP TABLE x;--' })).toBe('"reports"."daily""; DROP TABLE x;--"');
  });
  it("shows a disabled schedule's active run", () => {
    expect(scheduleStatus({ enabled: false, running_until: 2000, last_error: null }, 1000)).toBe("Running");
    expect(scheduleStatus({ enabled: false, running_until: null, last_error: null }, 1000)).toBe("Disabled");
    expect(scheduleStatus({ enabled: true, running_until: null, last_error: "failed" }, 1000)).toBe("Failed");
    expect(scheduleStatus({ enabled: true, running_until: 500, last_error: null }, 1000)).toBe("Recovering");
  });
});
