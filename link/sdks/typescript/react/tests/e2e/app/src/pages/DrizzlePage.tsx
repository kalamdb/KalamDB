import React from "react";
import { LiveQuery } from "@kalamdb/react";
import { asc, eq } from "drizzle-orm";
import { messages, schemaName_ } from "../schema";

export function DrizzlePage() {
  const roomId = new URLSearchParams(window.location.search).get("room") ?? "main";
  return (
    <div>
      <h1 data-testid="page-title">Drizzle mode</h1>
      <p data-testid="schema-name">{schemaName_}</p>
      <LiveQuery
        table={messages}
        where={(t) => eq(t.roomId, roomId)}
        orderBy={(t) => asc(t.createdAt)}
        deps={[roomId]}
      >
        {({ rows, state, insert }) => (
          <section>
            <p data-testid="status">{state.status}</p>
            <p data-testid="error">{state.error?.message ?? ""}</p>
            <p data-testid="row-count">{rows.length}</p>
            <ul>
              {rows.map((row) => (
                <li key={row.id} data-testid="row" data-id={row.id}>
                  <span data-testid="row-body">{row.body}</span>
                </li>
              ))}
            </ul>
            <button
              data-testid="add"
              disabled={state.inserting}
              onClick={() =>
                insert(messages).values({
                  id: crypto.randomUUID(),
                  roomId,
                  body: `body-${rows.length + 1}`,
                  createdAt: new Date(),
                })
              }
            >
              add
            </button>
          </section>
        )}
      </LiveQuery>
    </div>
  );
}
