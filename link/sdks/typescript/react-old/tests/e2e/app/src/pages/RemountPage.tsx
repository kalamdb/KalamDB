import React from "react";
import { LiveQuery } from "@kalamdb/react";
import { asc, eq } from "drizzle-orm";
import { messages, schemaName_ } from "../schema";

function Inner({ roomId }: { roomId: string }) {
  return (
    <LiveQuery
      table={messages}
      where={(t) => eq(t.roomId, roomId)}
      orderBy={(t) => asc(t.createdAt)}
      deps={[roomId]}
    >
      {({ rows, state }) => (
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
        </section>
      )}
    </LiveQuery>
  );
}

export function RemountPage() {
  const roomId = new URLSearchParams(window.location.search).get("room") ?? "main";
  const [version, setVersion] = React.useState(0);
  const [mounted, setMounted] = React.useState(true);

  return (
    <div>
      <h1 data-testid="page-title">Remount</h1>
      <p data-testid="schema-name">{schemaName_}</p>
      <button data-testid="toggle-mount" onClick={() => setMounted((m) => !m)}>
        toggle
      </button>
      <button data-testid="remount" onClick={() => setVersion((v) => v + 1)}>
        remount
      </button>
      {mounted ? <Inner key={version} roomId={roomId} /> : <p data-testid="unmounted">unmounted</p>}
    </div>
  );
}
