import React from "react";
import { useLiveQuery } from "@kalamdb/react";
import { asc, eq } from "drizzle-orm";
import { messages, schemaName_ } from "../schema";

export function SelectTransformPage() {
  const roomId = new URLSearchParams(window.location.search).get("room") ?? "main";
  const summary = useLiveQuery({
    table: messages,
    where: (t) => eq(t.roomId, roomId),
    orderBy: (t) => asc(t.createdAt),
    deps: [roomId],
    select: (c) => ({
      total: c.rows.length,
      first: c.rows[0]?.body ?? "",
      last: c.rows[c.rows.length - 1]?.body ?? "",
      loading: c.state.loading,
    }),
  });

  return (
    <div>
      <h1 data-testid="page-title">Select transform</h1>
      <p data-testid="schema-name">{schemaName_}</p>
      <p data-testid="loading">{summary.loading ? "yes" : "no"}</p>
      <p data-testid="total">{summary.total}</p>
      <p data-testid="first">{summary.first}</p>
      <p data-testid="last">{summary.last}</p>
    </div>
  );
}
