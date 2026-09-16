import { describe, expect, it, vi } from "vitest";
vi.mock("@/lib/api", () => ({ executeSql: vi.fn() }));
import { CREATE_SCHEDULE_SQL, formatScheduleTime, scheduleSqlName, scheduleStatus } from "./schedulesService";

describe("schedule monitoring", () => {
  it("quotes namespace and name independently", () => {
    expect(scheduleSqlName({ namespace_id: 'reports', name: 'daily"; DROP TABLE x;--' })).toBe('"reports"."daily""; DROP TABLE x;--"');
  });
  it("includes a ready-to-edit CREATE SCHEDULE template", () => {
    expect(CREATE_SCHEDULE_SQL).toContain("CREATE SCHEDULE");
    expect(CREATE_SCHEDULE_SQL).toContain("EXECUTE PROCEDURE");
    expect(CREATE_SCHEDULE_SQL).toContain("CRON");
  });
  it("formats catalog millis and RFC3339 log timestamps", () => {
    expect(formatScheduleTime(1_000)).toBe(new Date(1_000).toLocaleString());
    expect(formatScheduleTime("1789588620000")).toBe(new Date(1_789_588_620_000).toLocaleString());
    expect(formatScheduleTime("2026-09-16T12:00:00.000Z")).toBe(new Date("2026-09-16T12:00:00.000Z").toLocaleString());
    expect(formatScheduleTime(null)).toBe("—");
  });
  it("shows a disabled schedule's active run", () => {
    expect(scheduleStatus({ enabled: false, running_until: 2000, last_error: null }, 1000)).toBe("Running");
    expect(scheduleStatus({ enabled: false, running_until: null, last_error: null }, 1000)).toBe("Disabled");
    expect(scheduleStatus({ enabled: true, running_until: null, last_error: "failed" }, 1000)).toBe("Failed");
    expect(scheduleStatus({ enabled: true, running_until: 500, last_error: null }, 1000)).toBe("Recovering");
  });
});
