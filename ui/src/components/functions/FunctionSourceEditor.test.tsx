// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { FunctionSourceEditor } from "./FunctionSourceEditor";

vi.mock("@monaco-editor/react", () => ({
  default: ({
    value,
    language,
    height,
    options,
  }: {
    value?: string;
    language?: string;
    height?: number;
    options?: {
      wordWrap?: string;
      readOnly?: boolean;
      scrollbar?: { vertical?: string; horizontal?: string };
    };
  }) => (
    <div
      data-language={language}
      data-word-wrap={options?.wordWrap}
      data-read-only={options?.readOnly ? "true" : "false"}
      data-vertical-scrollbar={options?.scrollbar?.vertical}
      data-horizontal-scrollbar={options?.scrollbar?.horizontal}
      style={{ height, overflow: "auto" }}
    >
      <pre>{value}</pre>
    </div>
  ),
}));

describe("FunctionSourceEditor", () => {
  afterEach(() => {
    cleanup();
  });

  it("renders TypeScript source in a scrollable read-only editor", () => {
    render(<FunctionSourceEditor source={"const venueId = input.venue_id;\nreturn venueId;"} />);

    const editor = screen.getByTestId("function-inline-source-editor");
    const monaco = editor.firstElementChild as HTMLElement;

    expect(screen.getByText(/const venueId = input.venue_id/)).toBeTruthy();
    expect(monaco.getAttribute("data-language")).toBe("typescript");
    expect(monaco.getAttribute("data-word-wrap")).toBe("off");
    expect(monaco.getAttribute("data-read-only")).toBe("true");
    expect(monaco.getAttribute("data-vertical-scrollbar")).toBe("visible");
    expect(monaco.getAttribute("data-horizontal-scrollbar")).toBe("visible");
    expect(editor.className).toContain("overflow-hidden");
    expect(Number.parseInt(monaco.style.height, 10)).toBeGreaterThanOrEqual(128);
  });

  it("caps tall source at the max editor height so both scrollbars can appear", () => {
    const source = Array.from({ length: 80 }, (_, index) => `const line${index} = ${index};`).join("\n");
    render(<FunctionSourceEditor source={source} />);

    const editor = screen.getByTestId("function-inline-source-editor");
    const monaco = editor.firstElementChild as HTMLElement;
    expect(monaco.style.height).toBe("384px");
  });
});
