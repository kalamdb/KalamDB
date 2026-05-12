import React from "react";
import { LiveQueries } from "@kalamdb/react";
import { asc, eq } from "drizzle-orm";
import { counters, messages, schemaName_ } from "../schema";

export function MultiQueryPage() {
  const roomId = new URLSearchParams(window.location.search).get("room") ?? "main";
  return (
    <div>
      <h1 data-testid="page-title">Multi-query</h1>
      <p data-testid="schema-name">{schemaName_}</p>
      <LiveQueries
        queries={{
          messages: {
            table: messages,
            where: (t) => eq(t.roomId, roomId),
            orderBy: (t) => asc(t.createdAt),
            deps: [roomId],
          },
          counters: {
            table: counters,
            orderBy: (t) => asc(t.id),
          },
        }}
      >
        {(ctx) => (
          <section>
            <p data-testid="aggregate-loading">{ctx.state.loading ? "yes" : "no"}</p>
            <p data-testid="aggregate-connected">{ctx.state.connected ? "yes" : "no"}</p>
            <p data-testid="messages-count">{ctx.messages.rows.length}</p>
            <p data-testid="counters-count">{ctx.counters.rows.length}</p>
          </section>
        )}
      </LiveQueries>
    </div>
  );
}
