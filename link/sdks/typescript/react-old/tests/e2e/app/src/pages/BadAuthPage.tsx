import React from "react";
import { LiveQuery, KalamProvider } from "@kalamdb/react";
import { Auth, createClient } from "@kalamdb/client";
import { messages, schemaName_ } from "../schema";

const badClient = createClient({
  url: new URL("/kdb", window.location.origin).toString(),
  authProvider: async () => Auth.basic("admin", "totally-wrong-password"),
  disableCompression: true,
});

export function BadAuthPage() {
  return (
    <div>
      <h1 data-testid="page-title">Bad auth</h1>
      <p data-testid="schema-name">{schemaName_}</p>
      <KalamProvider client={badClient}>
        <LiveQuery table={messages}>
          {({ rows, state }) => (
            <section>
              <p data-testid="status">{state.status}</p>
              <p data-testid="row-count">{rows.length}</p>
              <p data-testid="error">{state.error?.message ?? ""}</p>
            </section>
          )}
        </LiveQuery>
      </KalamProvider>
    </div>
  );
}
