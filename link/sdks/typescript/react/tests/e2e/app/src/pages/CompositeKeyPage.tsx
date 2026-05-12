import React from "react";
import { LiveQuery } from "@kalamdb/react";
import { text, integer } from "drizzle-orm/pg-core";
import { asc, eq } from "drizzle-orm";
import { kTable } from "@kalamdb/orm";
import { schemaName_ } from "../schema";

const composite = kTable.user(`${schemaName_}.composite`, {
  id: text("id").primaryKey(),
  roomId: text("room_id").notNull(),
  messageId: text("message_id").notNull(),
  value: integer("value").notNull(),
});

export function CompositeKeyPage() {
  const roomId = new URLSearchParams(window.location.search).get("room") ?? "main";
  return (
    <div>
      <h1 data-testid="page-title">Composite key</h1>
      <p data-testid="schema-name">{schemaName_}</p>
      <LiveQuery
        table={composite}
        where={(t) => eq(t.roomId, roomId)}
        orderBy={(t) => asc(t.messageId)}
        getKey={["room_id", "message_id"]}
        deps={[roomId]}
      >
        {({ rows, state }) => (
          <section>
            <p data-testid="status">{state.status}</p>
            <p data-testid="row-count">{rows.length}</p>
            <ul>
              {rows.map((r) => (
                <li
                  key={`${r.roomId}:${r.messageId}`}
                  data-testid="row"
                  data-room={r.roomId}
                  data-msg={r.messageId}
                >
                  <span data-testid="row-value">{r.value}</span>
                </li>
              ))}
            </ul>
          </section>
        )}
      </LiveQuery>
    </div>
  );
}
