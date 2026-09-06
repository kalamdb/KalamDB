import { describe, expect, it } from "vitest";

import { formatTimestamp, toMilliseconds } from "./formatters";

describe("timestamp formatting", () => {
  it("treats SQL timestamp data types as microseconds", () => {
    const micros = 1_735_689_600_000_000;

    expect(formatTimestamp(micros, "Timestamp(Microsecond, None)", "iso8601-datetime", "utc"))
      .toBe("2025-01-01T00:00:00Z");
    expect(formatTimestamp(micros, "Timestamp", "iso8601-datetime", "utc"))
      .toBe("2025-01-01T00:00:00Z");
    expect(formatTimestamp(micros, "TIMESTAMP", "iso8601-datetime", "utc"))
      .toBe("2025-01-01T00:00:00Z");
    expect(formatTimestamp(micros, "Timestamp(Millisecond, None)", "iso8601-datetime", "utc"))
      .toBe("2025-01-01T00:00:00Z");
    expect(formatTimestamp(micros, "Timestamp(Nanosecond, None)", "iso8601-datetime", "utc"))
      .toBe("2025-01-01T00:00:00Z");
  });

  it("keeps explicit millisecond conversion for non-SQL values", () => {
    expect(toMilliseconds(1_735_689_600_000, "millisecond")).toBe(1_735_689_600_000);
  });
});
