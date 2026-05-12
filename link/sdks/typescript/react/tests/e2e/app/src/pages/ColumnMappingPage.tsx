import React from "react";
import { useLiveQuery } from "@kalamdb/react";
import { asc, eq } from "drizzle-orm";
import { messages } from "../schema";

export function ColumnMappingPage() {
  const roomId = new URLSearchParams(window.location.search).get("room") ?? "main";
  const ctx = useLiveQuery({
    table: messages,
    where: (t) => eq(t.roomId, roomId),
    orderBy: (t) => asc(t.createdAt),
    deps: [roomId],
  });

  return (
    <div>
      <h1 data-testid="page-title">Column mapping</h1>
      <p data-testid="status">{ctx.state.status}</p>
      <p data-testid="row-count">{ctx.rows.length}</p>
      <ul>
        {ctx.rows.map((r) => (
          <li key={r.id} data-testid="row">
            <span data-testid="row-author">{r.authorName ?? ""}</span>
            <span data-testid="row-body">{r.body}</span>
          </li>
        ))}
      </ul>
      <p data-testid="error">{ctx.state.error?.message ?? ""}</p>
      <button
        data-testid="add-with-camel"
        onClick={() =>
          ctx.insert(messages).values({
            id: crypto.randomUUID(),
            roomId,
            body: "from-camel",
            authorName: "Inas",
            createdAt: new Date(),
          })
        }
      >
        add
      </button>
    </div>
  );
}
