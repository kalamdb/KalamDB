import { describe, expect, it } from "vitest";
import {
  DEFAULT_CUSTOM,
  DEFAULT_NONE,
  TABLE_TYPE_OPTIONS,
  defaultPresetsForType,
} from "./types";

describe("table editor type metadata", () => {
  it("describes each creatable table type", () => {
    expect(
      TABLE_TYPE_OPTIONS.map(({ value, label, description }) => ({
        value,
        label,
        description,
      })),
    ).toEqual([
      {
        value: "user",
        label: "User",
        description: expect.stringContaining("per-user"),
      },
      {
        value: "shared",
        label: "Shared",
        description: expect.stringContaining("shared"),
      },
      {
        value: "stream",
        label: "Stream",
        description: expect.stringContaining("append"),
      },
    ]);
  });

  it("filters default presets by compatible column type", () => {
    expect(defaultPresetsForType("TIMESTAMP").map((p) => p.value)).toEqual([
      DEFAULT_NONE,
      "NOW()",
      DEFAULT_CUSTOM,
    ]);
    expect(defaultPresetsForType("TEXT").map((p) => p.value)).toEqual([
      DEFAULT_NONE,
      "ULID()",
      "UUID_V7()",
      DEFAULT_CUSTOM,
    ]);
    expect(defaultPresetsForType("BIGINT").map((p) => p.value)).toEqual([
      DEFAULT_NONE,
      "SNOWFLAKE_ID()",
      DEFAULT_CUSTOM,
    ]);
  });
});
