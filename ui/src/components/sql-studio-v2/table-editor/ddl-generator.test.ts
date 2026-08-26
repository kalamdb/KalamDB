import { describe, expect, it } from "vitest";
import { emptyDraft, tableToDraft } from "./types";
import { generateAlterTableSql, generateCreateTableSql } from "./ddl-generator";

describe("table editor DDL generator", () => {
  it("creates stream tables with stream-specific options", () => {
    const draft = emptyDraft("default", "stream");
    draft.name = "audit_stream";
    draft.options.ttlSeconds = "7200";
    draft.options.evictionStrategy = "hybrid";
    draft.options.maxStreamSizeBytes = "1048576";
    draft.options.compression = "zstd";

    const sql = generateCreateTableSql(draft);

    expect(sql).toContain("TYPE = 'STREAM'");
    expect(sql).toContain("TTL_SECONDS = 7200");
    expect(sql).toContain("EVICTION_STRATEGY = 'hybrid'");
    expect(sql).toContain("MAX_STREAM_SIZE_BYTES = 1048576");
    expect(sql).not.toContain("COMPRESSION");
  });

  it("alters only changed shared table options", () => {
    const original = tableToDraft({
      namespace: "default",
      name: "settings",
      tableType: "shared",
      storageId: "local",
      options: {
        table_type: "SHARED",
        storage_id: "local",
        flush_policy: { type: "row_limit", row_limit: 1000 },
        compression: "snappy",
      },
      columns: [
        {
          name: "id",
          dataType: "BIGINT",
          isNullable: false,
          isPrimaryKey: true,
        },
        {
          name: "value",
          dataType: "TEXT",
          isNullable: false,
          isPrimaryKey: false,
        },
      ],
    });
    const draft = structuredClone(original);
    draft.options.flushPolicyKind = "combined";
    draft.options.flushRows = "2000";
    draft.options.flushIntervalSeconds = "120";
    draft.options.compression = "none";

    const sql = generateAlterTableSql(original, draft);

    expect(sql).toContain("ALTER TABLE default.settings SET TBLPROPERTIES");
    expect(sql).not.toContain("ACCESS_LEVEL");
    expect(sql).toContain("FLUSH_POLICY = 'rows:2000,interval:120'");
    expect(sql).toContain("COMPRESSION = 'none'");
    expect(sql).not.toContain("STORAGE_ID");
  });

  it("creates shared table policies after CREATE TABLE", () => {
    const draft = emptyDraft("chat", "shared");
    draft.name = "documents";
    draft.policies = [
      {
        id: "p1",
        name: "owner_read",
        command: "select",
        targets: ["user", "service"],
        usingExpr: "owner_id = CURRENT_USER()",
        withCheckExpr: "",
        isNew: true,
        isDeleted: false,
      },
      {
        id: "p2",
        name: "owner_insert",
        command: "insert",
        targets: ["user"],
        usingExpr: "ignored",
        withCheckExpr: "owner_id = CURRENT_USER()",
        isNew: true,
        isDeleted: false,
      },
    ];

    const sql = generateCreateTableSql(draft);

    expect(sql).toContain("CREATE TABLE chat.documents");
    expect(sql).not.toContain("ACCESS_LEVEL");
    expect(sql).toContain(
      "CREATE POLICY owner_read ON chat.documents FOR SELECT TO user, service USING (owner_id = CURRENT_USER())",
    );
    expect(sql).toContain(
      "CREATE POLICY owner_insert ON chat.documents FOR INSERT TO user WITH CHECK (owner_id = CURRENT_USER())",
    );
    expect(sql).not.toMatch(/FOR INSERT[\s\S]*USING/);
  });

  it("alters, renames, creates, and drops shared table policies", () => {
    const original = tableToDraft({
      namespace: "chat",
      name: "documents",
      tableType: "shared",
      columns: [
        {
          name: "id",
          dataType: "BIGINT",
          isNullable: false,
          isPrimaryKey: true,
        },
      ],
      policies: [
        {
          name: "owner_read",
          command: "select",
          targets: [{ role: "user" }],
          usingSql: "owner_id = CURRENT_USER()",
        },
        {
          name: "stale_read",
          command: "select",
          targets: ["public"],
          usingSql: "true",
        },
      ],
    });
    const draft = structuredClone(original);
    const ownerRead = draft.policies[0];
    const staleRead = draft.policies[1];
    if (!ownerRead || !staleRead) throw new Error("expected seed policies");
    ownerRead.targets = ["user", "service"];
    ownerRead.usingExpr = "owner_id = CURRENT_USER() AND archived = false";
    ownerRead.name = "document_owner_read";
    staleRead.isDeleted = true;
    draft.policies.push({
      id: "p-new",
      name: "owner_write",
      command: "all",
      targets: ["user"],
      usingExpr: "owner_id = CURRENT_USER()",
      withCheckExpr: "owner_id = CURRENT_USER()",
      isNew: true,
      isDeleted: false,
    });

    const sql = generateAlterTableSql(original, draft);

    expect(sql).toContain("DROP POLICY stale_read ON chat.documents;");
    expect(sql).toContain(
      "ALTER POLICY owner_read ON chat.documents TO user, service USING (owner_id = CURRENT_USER() AND archived = false);",
    );
    expect(sql).toContain(
      "ALTER POLICY owner_read ON chat.documents RENAME TO document_owner_read;",
    );
    expect(sql).toContain(
      "CREATE POLICY owner_write ON chat.documents FOR ALL TO user USING (owner_id = CURRENT_USER()) WITH CHECK (owner_id = CURRENT_USER());",
    );
  });
});
