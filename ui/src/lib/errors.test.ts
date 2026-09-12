import { describe, expect, it } from "vitest";
import { getErrorMessage, toSerializableErrorPayload } from "@/lib/errors";

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

  it("unwraps JSON query error bodies into a readable message", () => {
    const body =
      '{"status":"error","results":[],"took":0.394,"error":{"code":"SQL_EXECUTION_ERROR","message":"procedure foo is not found"}}';

    expect(getErrorMessage(new Error(body), "fallback")).toBe(
      "SQL_EXECUTION_ERROR: procedure foo is not found",
    );
    expect(getErrorMessage(body, "fallback")).toBe(
      "SQL_EXECUTION_ERROR: procedure foo is not found",
    );
  });

  it("prefixes structured error details with the code", () => {
    expect(
      getErrorMessage(
        { code: "SQL_EXECUTION_ERROR", message: "relation public.missing does not exist" },
        "fallback",
      ),
    ).toBe("SQL_EXECUTION_ERROR: relation public.missing does not exist");
  });
});

describe("toSerializableErrorPayload", () => {
  it("parses JSON error messages from Error objects", () => {
    const body =
      '{"status":"error","results":[],"error":{"code":"SQL_EXECUTION_ERROR","message":"procedure foo is not found"}}';

    expect(toSerializableErrorPayload(new Error(body))).toEqual({
      status: "error",
      results: [],
      error: {
        code: "SQL_EXECUTION_ERROR",
        message: "procedure foo is not found",
      },
    });
  });

  it("keeps a plain Error serializable", () => {
    expect(toSerializableErrorPayload(new Error("boom"))).toEqual({
      name: "Error",
      message: "boom",
    });
  });
});