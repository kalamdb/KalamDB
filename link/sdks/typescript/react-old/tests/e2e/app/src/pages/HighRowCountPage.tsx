import React from "react";
import { LiveQuery } from "@kalamdb/react";
import { asc, eq } from "drizzle-orm";
import { messages, schemaName_ } from "../schema";

export function HighRowCountPage() {
  const roomId = new URLSearchParams(window.location.search).get("room") ?? "stress";
  return (
    <div>
      <h1 data-testid="page-title">High row count</h1>
      <p data-testid="schema-name">{schemaName_}</p>
      <LiveQuery
        table={messages}
        where={(t) => eq(t.roomId, roomId)}
        orderBy={(t) => asc(t.createdAt)}
        limit={500}
        deps={[roomId]}
      >
        {({ rows, state }) => (
          <section>
            <p data-testid="status">{state.status}</p>
            <p data-testid="row-count">{rows.length}</p>
          </section>
        )}
      </LiveQuery>
    </div>
  );
}
