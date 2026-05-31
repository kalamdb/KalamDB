// @vitest-environment jsdom

import { cleanup, render, screen } from "@testing-library/react";
import { fireEvent } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { SlowQueriesPanel } from "@/components/dashboard/SlowQueriesPanel";
import type { SlowQuery } from "@/services/systemTableService";

describe("SlowQueriesPanel", () => {
  afterEach(() => {
    cleanup();
  });

  it("renders slow queries with timestamp, SQL, duration, and colored priority", () => {
    const queries: SlowQuery[] = [
      {
        timestamp: "2026-05-28T11:00:00Z",
        timestamp_ms: 1_779_964_800_000,
        duration_ms: 250,
        user_id: "root",
        table_type: "user",
        table_name: "events",
        row_count: 1,
        query: "SELECT * FROM events LIMIT 100",
      },
      {
        timestamp: "2026-05-28T11:01:00Z",
        timestamp_ms: 1_779_964_860_000,
        duration_ms: 1500,
        user_id: "root",
        table_type: "user",
        table_name: "orders",
        row_count: 1,
        query: "SELECT * FROM orders WHERE status = 'pending'",
      },
      {
        timestamp: "2026-05-28T11:02:00Z",
        timestamp_ms: 1_779_964_920_000,
        duration_ms: 7200,
        user_id: "root",
        table_type: "user",
        table_name: "audit",
        row_count: 1,
        query: "SELECT * FROM audit_log ORDER BY timestamp DESC",
      },
    ];

    render(<SlowQueriesPanel queries={queries} />);

    expect(screen.getByText("Slow Queries")).toBeTruthy();
    expect(screen.getByText("Timestamp")).toBeTruthy();
    expect(screen.getByText("SQL statement")).toBeTruthy();
    expect(screen.getByText("Time took")).toBeTruthy();
    expect(screen.getByText("Priority")).toBeTruthy();
    expect(screen.getByText("SELECT * FROM events LIMIT 100")).toBeTruthy();
    expect(screen.getByText("SELECT * FROM orders WHERE status = 'pending'")).toBeTruthy();
    expect(screen.getByText("SELECT * FROM audit_log ORDER BY timestamp DESC")).toBeTruthy();
    expect(screen.getByText("250 ms")).toBeTruthy();
    expect(screen.getByText("1500 ms")).toBeTruthy();
    expect(screen.getByText("7200 ms")).toBeTruthy();
    expect(screen.getByText("Low")).toBeTruthy();
    expect(screen.getByText("Medium")).toBeTruthy();
    expect(screen.getByText("High")).toBeTruthy();
  });

  it("renders an empty state when no slow queries are available", () => {
    render(<SlowQueriesPanel queries={[]} />);

    expect(screen.getByText("No slow queries recorded.")).toBeTruthy();
  });

  it("shows latest slow queries first with bottom pagination navigator", () => {
    const queries: SlowQuery[] = Array.from({ length: 11 }, (_, index) => ({
      timestamp: `2026-05-28T11:${String(index).padStart(2, "0")}:00Z`,
      timestamp_ms: 1_779_964_800_000 + index * 60_000,
      duration_ms: 1200 + index,
      user_id: "root",
      table_type: "user",
      table_name: "events",
      row_count: 1,
      query: `SELECT ${index}`,
    }));

    render(<SlowQueriesPanel queries={queries} pageSize={10} />);

    expect(screen.getByText("Page 1 / 2")).toBeTruthy();
    expect(screen.getByText("SELECT 10")).toBeTruthy();
    expect(screen.queryByText("SELECT 0")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    expect(screen.getByText("Page 2 / 2")).toBeTruthy();
    expect(screen.getByText("SELECT 0")).toBeTruthy();
  });
});