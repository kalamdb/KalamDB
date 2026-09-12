// @vitest-environment jsdom

import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { CodeBlock } from "./code-block";

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

describe("CodeBlock", () => {
  it("renders nested KalamCellValue payloads with their underlying values", () => {
    render(
      <CodeBlock
        value={{
          type: "initial_data_batch",
          rows: [
            {
              _seq: {
                toJson: () => "9223372036854775807",
              },
              name: {
                toJson: () => "metric-a",
              },
            },
          ],
        }}
        jsonPreferred
      />,
    );

    expect(screen.getByText(/9223372036854775807/)).toBeTruthy();
    expect(screen.getByText(/metric-a/)).toBeTruthy();
  });

  it("wraps long JSON lines so they stay readable", () => {
    const { container } = render(
      <CodeBlock
        value={{
          code: "SQL_EXECUTION_ERROR",
          message: "procedure foo is not found",
        }}
        jsonPreferred
      />,
    );

    const pre = container.querySelector("pre");
    expect(pre?.className).toContain("whitespace-pre-wrap");
    expect(pre?.className).toContain("break-all");
    expect(screen.getByText(/SQL_EXECUTION_ERROR/)).toBeTruthy();
  });
});
