// @vitest-environment jsdom

import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { LIVE_META } from "@/features/sql-studio/state/sqlStudioWorkspaceSlice";
import { StudioResultsGrid } from "./StudioResultsGrid";

vi.mock("@kalamdb/client", () => ({
  KalamCellValue: class KalamCellValue {
    value: unknown;
    constructor(value: unknown) {
      this.value = value;
    }
    toJson() {
      return this.value;
    }
  },
}));

vi.mock("@/components/sql-preview", () => ({
  useSqlPreview: () => ({
    openPreview: vi.fn(),
  }),
}));

vi.mock("@/components/ui/toaster-provider", () => ({
  useToast: () => ({
    toast: vi.fn(),
  }),
}));

describe("StudioResultsGrid", () => {
  it("renders SQL action controls on the right side of the results header", () => {
    render(
      <StudioResultsGrid
        result={null}
        isRunning={false}
        isLiveMode={false}
        activeSql="select 1;"
        selectedTable={null}
        currentUsername="admin"
        resultView="results"
        onResultViewChange={vi.fn()}
        onRefreshAfterCommit={vi.fn()}
        actions={<button type="button">Run query</button>}
      />,
    );

    const header = screen.getByTestId("sql-results-header");

    expect(header.textContent).toContain("Results");
    expect(header.textContent).toContain("Run query");
  });

  it("renders Supabase-style one-line column headers and pager", () => {
    render(
      <StudioResultsGrid
        result={{
          status: "success",
          rows: [{ id: 1, alt: "hello" }],
          schema: [
            { name: "id", dataType: "int4", index: 0, isPrimaryKey: true },
            { name: "alt", dataType: "varchar", index: 1 },
          ],
          tookMs: 3,
          rowCount: 1,
          logs: [],
        }}
        isRunning={false}
        isLiveMode={false}
        activeSql="select id, alt from public.media;"
        selectedTable={null}
        currentUsername="admin"
        resultView="results"
        onResultViewChange={vi.fn()}
        onRefreshAfterCommit={vi.fn()}
      />,
    );

    expect(screen.getByTestId("results-column-header-id").textContent).toContain(
      "id int4",
    );
    expect(screen.getByTestId("results-column-header-alt").textContent).toContain(
      "alt varchar",
    );
    expect(screen.getAllByTitle("Resize column")).toHaveLength(2);
    expect((screen.getByLabelText("Results page") as HTMLInputElement).value).toBe("1");
    expect(screen.getByText("1 records")).toBeTruthy();
  });

  it("keeps live changes visible when the initial snapshot exceeds the render cap", () => {
    const initialRows = Array.from({ length: 1000 }, (_, index) => ({
      id: index + 1,
      email: `user${index + 1}@example.com`,
      _seq: index + 1,
      [LIVE_META.CHANGE_TYPE]: "initial",
      [LIVE_META.CHANGED_AT]: "2026-06-13T07:38:53.000Z",
      [LIVE_META.CHANGED_COLS]: "",
      [LIVE_META.BATCH_NUM]: 1,
    }));
    const insertedRow = {
      id: 1001,
      email: "new-user@example.com",
      _seq: 1001,
      [LIVE_META.CHANGE_TYPE]: "insert",
      [LIVE_META.CHANGED_AT]: new Date().toISOString(),
      [LIVE_META.CHANGED_COLS]: "",
      [LIVE_META.BATCH_NUM]: "",
    };

    render(
      <StudioResultsGrid
        result={{
          status: "success",
          rows: [...initialRows, insertedRow],
          schema: [
            { name: "id", dataType: "int", index: 0, isPrimaryKey: true },
            { name: "email", dataType: "text", index: 1 },
            { name: "_seq", dataType: "bigint", index: 2 },
            { name: LIVE_META.CHANGE_TYPE, dataType: "text", index: 3 },
            { name: LIVE_META.CHANGED_AT, dataType: "text", index: 4 },
            { name: LIVE_META.CHANGED_COLS, dataType: "text", index: 5 },
            { name: LIVE_META.BATCH_NUM, dataType: "text", index: 6 },
          ],
          tookMs: 0,
          rowCount: 1001,
          logs: [],
        }}
        isRunning={false}
        isLiveMode={true}
        activeSql="subscribe to select * from public.users;"
        selectedTable={null}
        currentUsername="admin"
        resultView="results"
        onResultViewChange={vi.fn()}
        onRefreshAfterCommit={vi.fn()}
      />,
    );

    expect(screen.getByText("new-user@example.com")).toBeTruthy();
    expect(screen.getByText("insert")).toBeTruthy();
  });

  it("draws selected cell borders on the table cell edge", () => {
    const { container } = render(
      <StudioResultsGrid
        result={{
          status: "success",
          rows: [{
            id: 232347,
            email: "user232347@example.com",
            _seq: 2,
            [LIVE_META.CHANGE_TYPE]: "insert",
            [LIVE_META.CHANGED_AT]: new Date().toISOString(),
            [LIVE_META.CHANGED_COLS]: "",
            [LIVE_META.BATCH_NUM]: "",
          }],
          schema: [
            { name: "id", dataType: "int", index: 0, isPrimaryKey: true },
            { name: "email", dataType: "text", index: 1 },
            { name: "_seq", dataType: "bigint", index: 2 },
            { name: LIVE_META.CHANGE_TYPE, dataType: "text", index: 3 },
            { name: LIVE_META.CHANGED_AT, dataType: "text", index: 4 },
            { name: LIVE_META.CHANGED_COLS, dataType: "text", index: 5 },
            { name: LIVE_META.BATCH_NUM, dataType: "text", index: 6 },
          ],
          tookMs: 0,
          rowCount: 1,
          logs: [],
        }}
        isRunning={false}
        isLiveMode={true}
        activeSql="subscribe to select * from public.users;"
        selectedTable={null}
        currentUsername="admin"
        resultView="results"
        onResultViewChange={vi.fn()}
        onRefreshAfterCommit={vi.fn()}
      />,
    );

    const cellContent = container.querySelector('[data-row-index="0"][data-column-name="id"]');
    expect(cellContent).not.toBeNull();

    fireEvent.click(cellContent!);

    const tableCell = cellContent!.closest("td");
    expect(tableCell?.className).toContain("ring-2");
    expect(cellContent!.className).not.toContain("ring-2");
  });
});
