// @vitest-environment jsdom

import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { CellDisplay } from "./index";

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

describe("CellDisplay", () => {
  it("renders integer values without thousands separators", () => {
    render(
      <CellDisplay
        value={776079}
        dataType="INT"
      />,
    );

    expect(screen.getByText("776079")).toBeTruthy();
  });

  it("renders bigint-like strings without losing precision", () => {
    render(
      <CellDisplay
        value="9223372036854775807"
        dataType="BIGINT"
      />,
    );

    expect(screen.getByText("9223372036854775807")).toBeTruthy();
  });

  it("renders SQL timestamp numbers instead of null", () => {
    render(
      <CellDisplay
        value={1_735_689_600_000_000}
        dataType="Timestamp"
      />,
    );

    expect(screen.queryByText("null")).toBeNull();
    expect(screen.getByText("2025-01-01T00:00:00Z")).toBeTruthy();
  });
});
