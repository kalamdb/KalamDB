// @vitest-environment jsdom

import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
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
});
