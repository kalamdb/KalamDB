import { describe, expect, it } from "vitest";

import { buildSqlCompletionData, resolveSqlContextualCompletions } from "./sqlCompletionCatalog";
import type { StudioNamespace } from "../shared/types";

describe("SQL Studio completion catalog", () => {
  it("always includes system.live with known columns", () => {
    const data = buildSqlCompletionData([]);

    expect(data.namespaces).toContain("system");
    expect(data.tablesByNamespace.system).toContain("live");
    expect(data.columnsByTable["system.live"]).toEqual(
      expect.arrayContaining(["live_id", "subscription_id", "namespace_id", "table_name", "user_id", "status"]),
    );
  });

  it("merges schema tables with static KalamDB and DataFusion completions", () => {
    const schema: StudioNamespace[] = [
      {
        database: "kalam",
        name: "app",
        tables: [
          {
            database: "kalam",
            namespace: "app",
            name: "messages",
            tableType: "user",
            columns: [
              { name: "id", dataType: "TEXT", isNullable: false, isPrimaryKey: true, ordinal: 0 },
              { name: "body", dataType: "TEXT", isNullable: true, isPrimaryKey: false, ordinal: 1 },
            ],
          },
        ],
      },
    ];

    const data = buildSqlCompletionData(schema);

    expect(data.tablesByNamespace.app).toContain("messages");
    expect(data.columnsByTable["app.messages"]).toEqual(["id", "body"]);
    expect(data.functions.map((entry) => entry.label)).toEqual(
      expect.arrayContaining(["CURRENT_USER()", "CURRENT_USER_ID()", "CURRENT_ROLE()", "NOW()", "COUNT()", "JSON_GET()"]),
    );
    expect(data.keywords).toEqual(expect.arrayContaining(["CALL", "SELECT"]));
    expect(data.snippets.map((entry) => entry.label)).toEqual(
      expect.arrayContaining(["CALL", "EXECUTE AS USER", "EXPLAIN", "DESCRIBE TABLE", "SUBSCRIBE TO", "KILL LIVE QUERY"]),
    );
  });

  it("resolves namespace table completions from the loaded explorer schema", () => {
    const schema: StudioNamespace[] = [
      {
        database: "kalam",
        name: "system",
        tables: [
          {
            database: "kalam",
            namespace: "system",
            name: "jobs",
            tableType: "system",
            columns: [
              { name: "job_id", dataType: "TEXT", isNullable: false, isPrimaryKey: true, ordinal: 0 },
            ],
          },
          {
            database: "kalam",
            namespace: "system",
            name: "users",
            tableType: "system",
            columns: [
              { name: "user_id", dataType: "TEXT", isNullable: false, isPrimaryKey: true, ordinal: 0 },
            ],
          },
        ],
      },
      {
        database: "kalam",
        name: "agent_events",
        tables: [
          {
            database: "kalam",
            namespace: "agent_events",
            name: "runs",
            tableType: "shared",
            columns: [
              { name: "id", dataType: "TEXT", isNullable: false, isPrimaryKey: true, ordinal: 0 },
              { name: "status", dataType: "TEXT", isNullable: false, isPrimaryKey: false, ordinal: 1 },
            ],
          },
          {
            database: "kalam",
            namespace: "agent_events",
            name: "messages",
            tableType: "shared",
            columns: [
              { name: "id", dataType: "TEXT", isNullable: false, isPrimaryKey: true, ordinal: 0 },
              { name: "body", dataType: "TEXT", isNullable: false, isPrimaryKey: false, ordinal: 1 },
            ],
          },
        ],
      },
    ];

    const data = buildSqlCompletionData(schema);

    expect(resolveSqlContextualCompletions(data, "system.", {})).toEqual({
      kind: "table",
      labels: expect.arrayContaining(["jobs", "users", "live"]),
      detail: "system table",
      partial: "",
    });

    expect(resolveSqlContextualCompletions(data, "SELECT *\nFROM agent_events.", {})).toEqual({
      kind: "table",
      labels: ["runs", "messages"],
      detail: "agent_events table",
      partial: "",
    });
  });

  it("resolves namespace-qualified and alias-qualified column completions", () => {
    const schema: StudioNamespace[] = [
      {
        database: "kalam",
        name: "agent_events",
        tables: [
          {
            database: "kalam",
            namespace: "agent_events",
            name: "runs",
            tableType: "shared",
            columns: [
              { name: "id", dataType: "TEXT", isNullable: false, isPrimaryKey: true, ordinal: 0 },
              { name: "status", dataType: "TEXT", isNullable: false, isPrimaryKey: false, ordinal: 1 },
            ],
          },
        ],
      },
    ];

    const data = buildSqlCompletionData(schema);

    expect(resolveSqlContextualCompletions(data, "SELECT agent_events.runs.", {})).toEqual({
      kind: "column",
      labels: ["id", "status"],
      detail: "agent_events.runs column",
      partial: "",
    });

    expect(
      resolveSqlContextualCompletions(
        data,
        "SELECT * FROM agent_events.runs AS r WHERE r.",
        { r: "agent_events.runs" },
      ),
    ).toEqual({
      kind: "column",
      labels: ["id", "status"],
      detail: "r alias column",
      partial: "",
    });
  });

  it("includes catalog procedures in completion data and refreshes when the catalog changes", () => {
    const first = buildSqlCompletionData([], [
      {
        id: "chat.send_message",
        schema: "chat",
        name: "send_message",
        signature: "conversation_id UUID, content TEXT",
        parameters: [],
        returnType: { kind: "builtin", builtin: "VOID", sqlName: "VOID", notNull: false, nonempty: false },
        returnTypeName: "VOID",
        security: "INVOKER",
        owner: "root",
        comment: null,
        implementation: "module",
        moduleId: "backend",
        revisionId: "backend:abc",
        grants: "dba",
      },
    ]);
    const second = buildSqlCompletionData([], [
      ...first.procedures,
      {
        ...first.procedures[0]!,
        id: "chat.join_room",
        name: "join_room",
      },
    ]);

    expect(first.procedureEntries.map((entry) => entry.label)).toEqual(["chat.send_message"]);
    expect(second.procedureEntries.map((entry) => entry.label)).toEqual(
      expect.arrayContaining(["chat.send_message", "chat.join_room"]),
    );
  });
});