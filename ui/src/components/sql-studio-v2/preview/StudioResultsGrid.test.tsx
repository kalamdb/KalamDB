// @vitest-environment jsdom

import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
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
});
