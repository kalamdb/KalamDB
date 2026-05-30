// @vitest-environment jsdom

import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { StudioResultsGrid } from "./StudioResultsGrid";

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
});
