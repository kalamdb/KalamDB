import React from "react";
import { LiveQuery } from "@kalamdb/react";
import { text } from "drizzle-orm/pg-core";
import { kTable } from "@kalamdb/orm";
import { schemaName_ } from "../schema";

const missing = kTable.user(`${schemaName_}.definitely_missing_table`, {
  id: text("id").primaryKey(),
});

export function MissingTablePage() {
  return (
    <div>
      <h1 data-testid="page-title">Missing table</h1>
      <p data-testid="schema-name">{schemaName_}</p>
      <LiveQuery table={missing}>
        {({ rows, state }) => (
          <section>
            <p data-testid="status">{state.status}</p>
            <p data-testid="row-count">{rows.length}</p>
            <p data-testid="error">{state.error?.message ?? ""}</p>
          </section>
        )}
      </LiveQuery>
    </div>
  );
}
