import React from "react";
import { LiveQueries } from "@kalamdb/react";
import { asc, eq } from "drizzle-orm";
import { messages, schemaName_ } from "../schema";
import { text } from "drizzle-orm/pg-core";
import { kTable } from "@kalamdb/orm";

const ghost = kTable.user("definitely_missing_schema_xyz.ghost", {
  id: text("id").primaryKey(),
});

export function PartialFailurePage() {
  const roomId = new URLSearchParams(window.location.search).get("room") ?? "main";
  return (
    <div>
      <h1 data-testid="page-title">Partial failure</h1>
      <p data-testid="schema-name">{schemaName_}</p>
      <LiveQueries
        queries={{
          good: {
            table: messages,
            where: (t) => eq(t.roomId, roomId),
            orderBy: (t) => asc(t.createdAt),
            deps: [roomId],
          },
          bad: {
            table: ghost,
          },
        }}
      >
        {(ctx) => (
          <section>
            <p data-testid="good-status">{ctx.good.state.status}</p>
            <p data-testid="good-count">{ctx.good.rows.length}</p>
            <p data-testid="bad-status">{ctx.bad.state.status}</p>
            <p data-testid="bad-error">{ctx.bad.state.error?.message ?? ""}</p>
            <p data-testid="aggregate-error">{ctx.state.error?.message ?? ""}</p>
          </section>
        )}
      </LiveQueries>
    </div>
  );
}
