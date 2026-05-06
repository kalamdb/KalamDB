import { describe, expect, it } from "vitest";
import { getErrorMessage } from "@/lib/errors";

describe("getErrorMessage", () => {
  it("returns the RTK custom error message used by unwrap rejections", () => {
    expect(
      getErrorMessage(
        { status: "CUSTOM_ERROR", error: "Statement 1 failed: Invalid operation" },
        "fallback",
      ),
    ).toBe("Statement 1 failed: Invalid operation");
  });

  it("includes nested backend details when present", () => {
    expect(
      getErrorMessage(
        {
          data: {
            error: {
              message: "Statement 1 failed: Invalid operation",
              details: "UPDATE system.users SET storage_mode = 'table'",
            },
          },
        },
        "fallback",
      ),
    ).toBe(
      "Statement 1 failed: Invalid operation\nUPDATE system.users SET storage_mode = 'table'",
    );
  });
});