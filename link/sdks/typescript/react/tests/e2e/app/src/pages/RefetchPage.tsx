import React from "react";
import { useLiveQuery } from "@kalamdb/react";
import { asc, eq } from "drizzle-orm";
import { messages, schemaName_ } from "../schema";

export function RefetchPage() {
  const roomId = new URLSearchParams(window.location.search).get("room") ?? "main";
  const { rows, state, refetch, insert } = useLiveQuery({
    table: messages,
    where: (t) => eq(t.roomId, roomId),
    orderBy: (t) => asc(t.createdAt),
    deps: [roomId],
  });
  return (
    <div>
      <h1 data-testid="page-title">Refetch</h1>
      <p data-testid="schema-name">{schemaName_}</p>
      <p data-testid="status">{state.status}</p>
      <p data-testid="row-count">{rows.length}</p>
      <button data-testid="add" onClick={() => insert(messages).values({ id: crypto.randomUUID(), roomId, body: "x", createdAt: new Date() })}>add</button>
      <button data-testid="refetch" onClick={() => { void refetch(); }}>refetch</button>
    </div>
  );
}
