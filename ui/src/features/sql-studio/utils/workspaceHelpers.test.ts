import { describe, expect, it } from "vitest";
import { prepareLiveSubscriptionSql } from "./workspaceHelpers";

describe("prepareLiveSubscriptionSql", () => {
  it("strips a top-level ORDER BY and says the grid can still sort", () => {
    const prepared = prepareLiveSubscriptionSql(
      "SELECT * FROM home_check.users ORDER BY created_at DESC LIMIT 100;",
    );

    expect(prepared.sql).not.toMatch(/order by/i);
    expect(prepared.sql).not.toMatch(/limit 100/i);
    expect(prepared.sql.toLowerCase()).toContain("select * from home_check.users");
    expect(prepared.notice).toMatch(/ORDER BY/);
  });

  it("keeps ORDER BY that is nested or inside a quoted identifier", () => {
    const nested = prepareLiveSubscriptionSql(
      "SELECT * FROM (SELECT id FROM users ORDER BY id) AS recent;",
    );
    expect(nested.sql.toLowerCase()).toContain("order by id");
    expect(nested.notice).toBeNull();

    const quoted = prepareLiveSubscriptionSql('SELECT "ORDER BY" FROM users;');
    expect(quoted.sql).toContain('"ORDER BY"');
    expect(quoted.notice).toBeNull();
  });
});
