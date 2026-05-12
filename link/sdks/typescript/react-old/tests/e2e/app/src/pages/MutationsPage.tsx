import React from "react";
import { useLiveQuery } from "@kalamdb/react";
import { asc, eq } from "drizzle-orm";
import { messages, schemaName_ } from "../schema";

export function MutationsPage() {
  const roomId = new URLSearchParams(window.location.search).get("room") ?? "main";
  const { rows, state, insert, update, remove, clearError } = useLiveQuery({
    table: messages,
    where: (t) => eq(t.roomId, roomId),
    orderBy: (t) => asc(t.createdAt),
    deps: [roomId],
  });

  const newId = React.useRef<string | null>(null);

  return (
    <div>
      <h1 data-testid="page-title">Mutations</h1>
      <p data-testid="schema-name">{schemaName_}</p>
      <p data-testid="status">{state.status}</p>
      <p data-testid="inserting">{state.inserting ? "yes" : "no"}</p>
      <p data-testid="updating-count">{state.updating.size}</p>
      <p data-testid="deleting-count">{state.deleting.size}</p>
      <p data-testid="error">{state.error?.message ?? ""}</p>
      <p data-testid="row-count">{rows.length}</p>
      <ul>
        {rows.map((row) => (
          <li key={row.id} data-testid="row" data-id={row.id}>
            <span data-testid="row-body">{row.body}</span>
            <span data-testid="row-updating">{state.updating.has(row.id) ? "yes" : "no"}</span>
            <span data-testid="row-deleting">{state.deleting.has(row.id) ? "yes" : "no"}</span>
            <button data-testid="edit" onClick={() => update(messages, row.id).set({ body: `${row.body}!` })}>edit</button>
            <button data-testid="del" onClick={() => remove(messages, row.id)}>del</button>
          </li>
        ))}
      </ul>
      <button
        data-testid="add"
        onClick={async () => {
          const id = crypto.randomUUID();
          newId.current = id;
          await insert(messages).values({ id, roomId, body: `body-${rows.length + 1}`, createdAt: new Date() });
        }}
      >
        add
      </button>
      <button data-testid="clear-error" onClick={clearError}>clear</button>
    </div>
  );
}
