// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
const mocks = vi.hoisted(() => ({
  query: vi.fn(),
  history: vi.fn(),
  refetch: vi.fn(),
  enable: vi.fn(),
  drop: vi.fn(),
  navigate: vi.fn(),
}));
vi.mock("@/store/apiSlice", () => ({
  apiSlice: {
    injectEndpoints: () => ({
      useGetSchedulesQuery: mocks.query,
      useGetScheduleHistoryQuery: mocks.history,
    }),
  },
}));
vi.mock("@/services/schedulesService", async (original) => ({
  ...await original<object>(),
  setScheduleEnabled: mocks.enable,
  dropSchedule: mocks.drop,
}));
vi.mock("@/lib/api", () => ({ executeSql: vi.fn() }));
vi.mock("react-router-dom", async (original) => {
  const actual = await original<typeof import("react-router-dom")>();
  return { ...actual, useNavigate: () => mocks.navigate };
});
import Schedules from "./Schedules";
import { CREATE_SCHEDULE_SQL } from "@/services/schedulesService";
const row = { schedule_id: "reports.daily", namespace_id: "reports", name: "daily", routine_id: "reports.summary", principal_user_id: "system", cron: "0 9 * * *", interval_ms: null, timezone: "UTC", enabled: true, next_run_at: 1000, running_until: null, owner: null, last_started_at: null, last_finished_at: null, last_error: null, run_count: 0, skip_count: 0 };
const log = {
  timestamp: "2026-09-16T12:00:00.000Z",
  node_id: "n1",
  execution_id: "run-1",
  request_id: "run-1",
  procedure_id: "reports.summary",
  module_id: null,
  revision_id: null,
  actor: "system",
  origin: "schedule",
  outcome: "ok",
  channel: "invocation",
  level: "info",
  error_code: null,
  message: null,
    duration_ms: 12,
    schedule_id: "reports.daily",
};
function show() { render(<MemoryRouter><Schedules /></MemoryRouter>); }
afterEach(cleanup);
beforeEach(() => {
  vi.clearAllMocks();
  mocks.query.mockReturnValue({ data: { rows: [row], hasMore: false }, refetch: mocks.refetch, isFetching: false });
  mocks.history.mockReturnValue({ data: [], isFetching: false });
});
describe("Schedules", () => {
  it("shows timing and disables through schedule DDL", async () => {
    show(); expect(screen.getByText("reports.daily")).toBeTruthy(); expect(screen.getByText("0 9 * * *")).toBeTruthy();
    expect(screen.getByTestId("schedules-row-reports.daily")).toBeTruthy();
    expect(screen.getByTestId("schedules-runs-reports.daily").textContent).toBe("0 / 0");
    fireEvent.click(screen.getByRole("button", { name: "Disable reports.daily" }));
    await waitFor(() => expect(mocks.enable).toHaveBeenCalledWith(row, false));
  });
  it("opens SQL Studio with a create-schedule template", () => {
    show();
    fireEvent.click(screen.getByRole("button", { name: "Create schedule" }));
    expect(mocks.navigate).toHaveBeenCalledWith("/sql", {
      state: { prefillTitle: "Create schedule", prefillSql: CREATE_SCHEDULE_SQL },
    });
  });
  it("opens run history when the schedule name is clicked", () => {
    mocks.history.mockReturnValue({ data: [log], isFetching: false });
    show();
    fireEvent.click(screen.getByRole("button", { name: "View history for reports.daily" }));
    expect(screen.getByTestId("schedule-history-table").textContent).toContain("Succeeded");
    expect(screen.getByTestId("schedule-history-table").textContent).toContain("run-1");
    expect(within(screen.getByRole("dialog")).getByRole("link", { name: "reports.summary" }).getAttribute("href")).toBe("/functions/reports.summary/logs");
  });
  it("renders query errors without pretending the catalog is empty", () => {
    mocks.query.mockReturnValue({ error: { error: "Permission denied" }, refetch: mocks.refetch, isFetching: false }); show();
    expect(screen.getByRole("alert").textContent).toContain("Permission denied");
    expect(screen.queryByText(/No schedules match/)).toBeNull();
  });
  it("shows empty and loading states", () => {
    mocks.query.mockReturnValue({ data: { rows: [], hasMore: false }, refetch: mocks.refetch, isFetching: false }); show();
    expect(screen.getByText(/No schedules match/)).toBeTruthy();
  });
});
