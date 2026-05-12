import React from "react";
import { useLiveQuery, useLiveSelection } from "@kalamdb/react";
import { asc, eq } from "drizzle-orm";
import { messages } from "../schema";

export function SelectionPage() {
  const roomId = new URLSearchParams(window.location.search).get("room") ?? "main";
  const ctx = useLiveQuery({
    table: messages,
    where: (t) => eq(t.roomId, roomId),
    orderBy: (t) => asc(t.createdAt),
    deps: [roomId],
  });

  const view = useLiveSelection(ctx, (c) => ({
    bodies: c.rows.map((r) => r.body),
    longestBody: c.rows.reduce((acc, r) => (r.body.length > acc.length ? r.body : acc), ""),
    total: c.rows.length,
  }));

  return (
    <div>
      <h1 data-testid="page-title">Selection</h1>
      <p data-testid="total">{view.total}</p>
      <p data-testid="longest">{view.longestBody}</p>
      <ul>
        {view.bodies.map((b, i) => (
          <li key={i} data-testid="body">{b}</li>
        ))}
      </ul>
      <button
        data-testid="add"
        onClick={() =>
          ctx.insert(messages).values({
            id: crypto.randomUUID(),
            roomId,
            body: `body-of-length-${view.total + 1}`.padEnd(view.total + 10, "x"),
            createdAt: new Date(),
          })
        }
      >
        add
      </button>
    </div>
  );
}
