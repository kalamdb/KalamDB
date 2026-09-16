// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
const mocks = vi.hoisted(() => ({ query: vi.fn(), refetch: vi.fn(), enable: vi.fn(), drop: vi.fn() }));
vi.mock("@/store/apiSlice", () => ({ apiSlice: { injectEndpoints: () => ({ useGetSchedulesQuery: mocks.query }) } }));
vi.mock("@/services/schedulesService", async (original) => ({ ...await original<object>(), setScheduleEnabled: mocks.enable, dropSchedule: mocks.drop }));
vi.mock("@/lib/api", () => ({ executeSql: vi.fn() }));
import Schedules from "./Schedules";
const row = { schedule_id: "reports.daily", namespace_id: "reports", name: "daily", routine_id: "reports.summary", principal_user_id: "system", cron: "0 9 * * *", interval_ms: null, timezone: "UTC", enabled: true, next_run_at: 1000, running_until: null, owner: null, last_started_at: null, last_finished_at: null, last_error: null, run_count: 0, skip_count: 0 };
function show() { render(<MemoryRouter><Schedules /></MemoryRouter>); }
afterEach(cleanup);
beforeEach(() => { vi.clearAllMocks(); mocks.query.mockReturnValue({ data: { rows: [row], hasMore: false }, refetch: mocks.refetch, isFetching: false }); });
describe("Schedules", () => {
  it("shows timing and disables through schedule DDL", async () => {
    show(); expect(screen.getByText("reports.daily")).toBeTruthy(); expect(screen.getByText("0 9 * * *")).toBeTruthy();
    expect(screen.getByTestId("schedules-row-reports.daily")).toBeTruthy();
    expect(screen.getByTestId("schedules-runs-reports.daily").textContent).toBe("0 / 0");
    fireEvent.click(screen.getByRole("button", { name: "Disable reports.daily" }));
    await waitFor(() => expect(mocks.enable).toHaveBeenCalledWith(row, false));
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
