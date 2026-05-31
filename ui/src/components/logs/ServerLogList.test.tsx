// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ServerLogList } from "./ServerLogList";

const mockUseGetServerLogsQuery = vi.fn();

vi.mock("@/store/apiSlice", () => ({
  useGetServerLogsQuery: (...args: unknown[]) => mockUseGetServerLogsQuery(...args),
}));

afterEach(() => {
  cleanup();
  mockUseGetServerLogsQuery.mockReset();
});

function createLogs(count: number) {
  return Array.from({ length: count }, (_, index) => {
    const timestamp = new Date(Date.UTC(2026, 4, 31, 12, 0, 0) - index * 60_000).toISOString();
    return {
      timestamp,
      level: "INFO",
      thread: "main",
      target: "kalamdb::server",
      line: "42",
      message: `Server log ${index + 1}`,
    };
  });
}

function renderServerLogs(logs = createLogs(50)) {
  mockUseGetServerLogsQuery.mockImplementation(() => ({
    data: logs,
    isLoading: false,
    error: null,
    refetch: vi.fn(),
  }));

  render(
    <MemoryRouter>
      <ServerLogList />
    </MemoryRouter>,
  );
}

describe("ServerLogList", () => {
  it("renders inline log columns and a visible frequency graph", () => {
    renderServerLogs();

    const header = screen.getByText("Timestamp").parentElement;
    expect(header?.style.gridTemplateColumns).toContain("minmax(190px, 220px)");
    expect(screen.getByText("Log Frequency Graph")).toBeTruthy();
    expect(document.querySelector('[title$="logs"]')).toBeTruthy();
  });

  it("uses the oldest timestamp as the cursor when moving to older logs", async () => {
    const logs = createLogs(50);
    renderServerLogs(logs);

    fireEvent.click(screen.getByRole("button", { name: /next \(older\)/i }));

    await waitFor(() => {
      const calls = mockUseGetServerLogsQuery.mock.calls;
      const lastCall = calls[calls.length - 1];
      expect(lastCall?.[0]).toMatchObject({
        beforeTimestamp: logs[logs.length - 1].timestamp,
      });
    });
  });
});
