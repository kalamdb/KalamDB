// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { StudioExecutionLog } from "./StudioExecutionLog";
import type { QueryLogEntry } from "../../shared/types";

vi.mock("@kalamdb/client", () => {
  class KalamCellValue {
    private value: unknown;

    constructor(value: unknown) {
      this.value = value;
    }

    static from(value: unknown) {
      return new KalamCellValue(value);
    }

    toJson() {
      return this.value;
    }
  }

  return { KalamCellValue };
});

describe("StudioExecutionLog", () => {
  beforeEach(() => {
    vi.stubGlobal(
      "ResizeObserver",
      class ResizeObserver {
        observe() {}
        unobserve() {}
        disconnect() {}
      },
    );
  });

  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
  });

  it("shows websocket payloads in the right-hand inspector and updates when another entry is selected", () => {
    const logs: QueryLogEntry[] = [
      {
        id: "send-1",
        level: "info",
        message: "WS SEND · subscribe",
        response: { raw: '{"type":"subscribe","sql":"SELECT id, name FROM default.events"}' },
        createdAt: "2026-04-13T14:22:01.380Z",
      },
      {
        id: "receive-1",
        level: "info",
        message: "WS RECEIVE · subscription_ack",
        response: { raw: '{"type":"subscription_ack","subscription_id":"sub-1"}' },
        createdAt: "2026-04-13T14:22:01.442Z",
      },
    ];

    render(
      <div className="h-[640px]">
        <StudioExecutionLog logs={logs} status="success" />
      </div>,
    );

    const timeline = screen.getByText(/timeline stream/i).closest("section");
    const details = screen.getByText(/raw payload and structured detail for the selected entry/i).closest("section");

    expect(timeline).toBeTruthy();
    expect(details).toBeTruthy();

    expect(within(details as HTMLElement).getByRole("heading", { name: /subscription_ack/i })).toBeTruthy();
    expect(within(details as HTMLElement).getByText(/sub-1/i)).toBeTruthy();

    const timelineButtons = within(timeline as HTMLElement).getAllByRole("button");
    fireEvent.click(timelineButtons[0]);

    expect(within(details as HTMLElement).getByText(/select id, name from default.events/i)).toBeTruthy();
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});