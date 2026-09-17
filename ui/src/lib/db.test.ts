import { describe, expect, it } from "vitest";
import { sqlWithInlineParams } from "@/lib/db";

describe("sqlWithInlineParams", () => {
  it("inlines numbered parameters from highest index to lowest", () => {
    expect(sqlWithInlineParams("select * from t where n = $1 limit $2", [10, 50])).toBe(
      "select * from t where n = 10 limit 50",
    );
    expect(sqlWithInlineParams("select * from t where name = $1", ["O'Reilly"])).toBe(
      "select * from t where name = 'O''Reilly'",
    );
  });
});
