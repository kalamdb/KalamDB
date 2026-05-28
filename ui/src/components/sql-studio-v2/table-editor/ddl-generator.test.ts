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
      accessLevel: "PRIVATE",
      options: {
        table_type: "SHARED",
        storage_id: "local",
        access_level: "private",
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
    draft.options.accessLevel = "public";
    draft.options.flushPolicyKind = "combined";
    draft.options.flushRows = "2000";
    draft.options.flushIntervalSeconds = "120";
    draft.options.compression = "none";

    const sql = generateAlterTableSql(original, draft);

    expect(sql).toContain("ALTER TABLE default.settings SET TBLPROPERTIES");
    expect(sql).toContain("ACCESS_LEVEL = 'PUBLIC'");
    expect(sql).toContain("FLUSH_POLICY = 'rows:2000,interval:120'");
    expect(sql).toContain("COMPRESSION = 'none'");
    expect(sql).not.toContain("STORAGE_ID");
  });
});
