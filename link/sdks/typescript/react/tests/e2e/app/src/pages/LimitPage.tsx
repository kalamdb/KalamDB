import React from "react";
import { LiveQuery } from "@kalamdb/react";
import { asc, eq } from "drizzle-orm";
import { messages, schemaName_ } from "../schema";

export function LimitPage() {
  const roomId = new URLSearchParams(window.location.search).get("room") ?? "main";
  return (
    <div>
      <h1 data-testid="page-title">Limit</h1>
      <p data-testid="schema-name">{schemaName_}</p>
      <LiveQuery
        table={messages}
        where={(t) => eq(t.roomId, roomId)}
        orderBy={(t) => asc(t.createdAt)}
        limit={3}
        deps={[roomId]}
      >
        {({ rows, state, insert }) => (
          <section>
            <p data-testid="status">{state.status}</p>
            <p data-testid="row-count">{rows.length}</p>
            <ul>
              {rows.map((r) => (
                <li key={r.id} data-testid="row" data-id={r.id}>
                  <span data-testid="row-body">{r.body}</span>
                </li>
              ))}
            </ul>
            <button
              data-testid="add"
              onClick={() =>
                insert(messages).values({
                  id: crypto.randomUUID(),
                  roomId,
                  body: `body-${Date.now()}`,
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
