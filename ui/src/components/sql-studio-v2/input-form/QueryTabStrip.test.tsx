// @vitest-environment jsdom

import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { QueryTabStrip } from "./QueryTabStrip";
import type { QueryTab } from "../shared/types";

const baseTab: QueryTab = {
  id: "tab-1",
  title: "Query one",
  sql: "select 1;",
  isDirty: false,
  unreadChangeCount: 0,
  isLive: false,
  liveStatus: "idle",
  resultView: "results",
  lastSavedAt: "2026-05-30T12:00:00.000Z",
  savedQueryId: "saved-1",
  subscriptionOptions: undefined,
};

describe("QueryTabStrip", () => {
  it("shows saved state in the tab title and renames from a double click", () => {
    const onRenameTab = vi.fn();

    render(
      <QueryTabStrip
        tabs={[baseTab]}
        activeTabId={baseTab.id}
        onTabSelect={vi.fn()}
        onAddTab={vi.fn()}
        onCloseTab={vi.fn()}
        onRenameTab={onRenameTab}
      />,
    );

    expect(screen.getByText("Saved")).toBeTruthy();

    fireEvent.doubleClick(screen.getByRole("button", { name: /query one/i }));
    const input = screen.getByDisplayValue("Query one");
    fireEvent.change(input, { target: { value: "Connection check" } });
    fireEvent.blur(input);

    expect(onRenameTab).toHaveBeenCalledWith(baseTab.id, "Connection check");
  });

  it("shows draft state when a tab has unsaved edits", () => {
    render(
      <QueryTabStrip
        tabs={[{ ...baseTab, isDirty: true }]}
        activeTabId={baseTab.id}
        onTabSelect={vi.fn()}
        onAddTab={vi.fn()}
        onCloseTab={vi.fn()}
        onRenameTab={vi.fn()}
      />,
    );

    expect(screen.getByText("Draft")).toBeTruthy();
  });

  it("keeps the live indicator in the same row as the tab title", () => {
    render(
      <QueryTabStrip
        tabs={[{ ...baseTab, isLive: true, liveStatus: "connected" }]}
        activeTabId={baseTab.id}
        onTabSelect={vi.fn()}
        onAddTab={vi.fn()}
        onCloseTab={vi.fn()}
        onRenameTab={vi.fn()}
      />,
    );

    const liveIndicator = screen.getByLabelText("Live query connected");
    const titleRow = liveIndicator.parentElement;

    expect(titleRow?.className).toContain("whitespace-nowrap");
    expect(titleRow?.textContent).toContain("Query one");
    expect(titleRow?.textContent).toContain("Saved");
  });
});
