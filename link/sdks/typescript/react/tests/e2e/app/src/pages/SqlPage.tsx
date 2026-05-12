import React from "react";
import { LiveQuery } from "@kalamdb/react";
import { schemaName_ } from "../schema";

export function SqlPage() {
  const roomId = new URLSearchParams(window.location.search).get("room") ?? "main";
  const query = `SELECT id, body FROM ${schemaName_}.messages WHERE room_id = '${roomId}' ORDER BY created_at ASC LIMIT 200`;
  return (
    <div>
      <h1 data-testid="page-title">SQL mode</h1>
      <p data-testid="schema-name">{schemaName_}</p>
      <LiveQuery query={query} getKey="id">
        {({ rows, state, insert }) => (
          <section>
            <p data-testid="status">{state.status}</p>
            <p data-testid="row-count">{rows.length}</p>
            {state.error ? <p data-testid="error">{state.error.message}</p> : null}
            <ul>
              {rows.map((row, i) => (
                <li key={i} data-testid="row">
                  <span data-testid="row-body">
                    {typeof row.body === "object" && row.body !== null && "asString" in row.body
                      ? (row.body as { asString: () => string }).asString()
                      : String(row.body)}
                  </span>
                </li>
              ))}
            </ul>
            <button
              data-testid="add"
              disabled={state.inserting}
              onClick={() =>
                insert(`${schemaName_}.messages`, {
                  id: crypto.randomUUID(),
                  room_id: roomId,
                  body: `sql-body-${rows.length + 1}`,
                  created_at: new Date().toISOString(),
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
