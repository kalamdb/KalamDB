import React from "react";
import { useLiveQuery } from "@kalamdb/react";
import { asc, eq } from "drizzle-orm";
import { messages, schemaName_ } from "../schema";
import { getClient } from "../client-setup";

export function DisconnectPage() {
  const roomId = new URLSearchParams(window.location.search).get("room") ?? "main";
  const { rows, state, refetch } = useLiveQuery({
    table: messages,
    where: (t) => eq(t.roomId, roomId),
    orderBy: (t) => asc(t.createdAt),
    deps: [roomId],
  });

  return (
    <div>
      <h1 data-testid="page-title">Disconnect</h1>
      <p data-testid="schema-name">{schemaName_}</p>
      <p data-testid="status">{state.status}</p>
      <p data-testid="row-count">{rows.length}</p>
      <p data-testid="error">{state.error?.message ?? ""}</p>
      <button
        data-testid="disconnect"
        onClick={async () => {
          await getClient().disconnect();
        }}
      >
        disconnect
      </button>
      <button data-testid="refetch" onClick={() => { void refetch(); }}>refetch</button>
    </div>
  );
}
