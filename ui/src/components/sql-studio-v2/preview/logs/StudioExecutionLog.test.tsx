// @vitest-environment jsdom

import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { StudioExecutionLog } from "./StudioExecutionLog";
import type { QueryLogEntry } from "../../shared/types";

const logs: QueryLogEntry[] = [
  {
    id: "log-1",
    level: "info",
    message: "WS SEND · subscribe",
    createdAt: "2026-05-31T12:00:00.000Z",
    response: { raw: { type: "subscribe" } },
  },
  {
    id: "log-2",
    level: "info",
    message: "WS RECEIVE · initial_data_batch",
    createdAt: "2026-05-31T12:00:01.000Z",
    response: { raw: { type: "initial_data_batch" } },
  },
];

describe("StudioExecutionLog", () => {
  afterEach(() => {
    cleanup();
  });
  it("renders the timeline and preview as a two-column split", () => {
    render(<StudioExecutionLog logs={logs} status="success" />);

    const split = screen.getByTestId("studio-execution-log-split");
    const timeline = screen.getByLabelText("Trace timeline");

    expect(split.className).toContain("flex-row");
    expect(timeline.style.width).toBe("40%");
    expect(timeline.style.minWidth).toBe("280px");
    expect(timeline.style.maxWidth).toBe("500px");
    expect(screen.getByLabelText("Trace details")).toBeTruthy();
  });

  it("wraps error messages and shows the structured payload instead of truncating", () => {
    const body =
      '{"status":"error","results":[],"took":0.394,"error":{"code":"SQL_EXECUTION_ERROR","message":"procedure foo is not found"}}';

    render(
      <StudioExecutionLog
        logs={[
          {
            id: "log-error",
            level: "error",
            message: body,
            createdAt: "2026-05-31T12:00:02.000Z",
            asUser: "admin",
            response: {},
          },
        ]}
        status="error"
      />,
    );

    const heading = screen.getByRole("heading", {
      name: "SQL_EXECUTION_ERROR: procedure foo is not found",
    });
    expect(heading.className).toContain("whitespace-pre-wrap");
    expect(heading.className).toContain("break-all");
    expect(heading.className).not.toContain("truncate");

    const details = screen.getByLabelText("Trace details");
    expect(details.textContent).toContain("SQL_EXECUTION_ERROR");
    expect(details.textContent).toContain("procedure foo is not found");
    expect(details.textContent).not.toMatch(/\{\s*\}/);
  });
});
