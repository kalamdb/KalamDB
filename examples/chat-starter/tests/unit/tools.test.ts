import { test } from "node:test";
import assert from "node:assert/strict";
import { dispatchTool, type ToolContext } from "../../src/agent/tools.js";
import { logger } from "../../src/lib/logger.js";

// Minimal stub of the KalamDBClient surface that tools.ts actually uses.
function makeStubClient(
  opts: {
    queryResult?: unknown;
    queryThrows?: Error;
    inserts?: Array<{ table: string; row: Record<string, unknown> }>;
    queries?: Array<{ sql: string; params?: unknown[] }>;
  } = {},
) {
  const inserts = opts.inserts ?? [];
  const queries = opts.queries ?? [];
  return {
    inserts,
    queries,
    client: {
      insert: async (table: string, row: Record<string, unknown>) => {
        inserts.push({ table, row });
      },
      query: async (sql: string, params?: unknown[]) => {
        queries.push({ sql, params });
        if (opts.queryThrows) throw opts.queryThrows;
        return opts.queryResult ?? { results: [{ named_rows: [] }] };
      },
      // unused but typed
      live: async () => () => undefined,
      update: async () => undefined,
      delete: async () => undefined,
    } as never,
  };
}

function makeCtx(client: ToolContext["client"]): ToolContext {
  return {
    client,
    log: logger.child({ component: "test" }),
    task: {
      id: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
      conversation_id: "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
      message_id: "cccccccc-cccc-cccc-cccc-cccccccccccc",
    },
    signal: new AbortController().signal,
    lastToolCallName: null,
    lastApprovalDecision: null,
  };
}

// =============================================================================
// query_database
// =============================================================================

test("query_database returns the row count + rows as JSON", async () => {
  const stub = makeStubClient({
    queryResult: { results: [{ named_rows: [{ n: 5 }] }] },
  });
  const ctx = makeCtx(stub.client);
  const out = await dispatchTool(ctx, {
    id: "c1",
    name: "query_database",
    arguments: { sql: "SELECT count(*) AS n FROM chat.conversations" },
  });
  const parsed = JSON.parse(out) as { row_count: number; rows: unknown[] };
  assert.equal(parsed.row_count, 1);
  assert.deepEqual(parsed.rows[0], { n: 5 });
  // Guard should have appended LIMIT 200.
  assert.match(stub.queries[0]!.sql, /LIMIT 200$/);
});

test("query_database rejects non-SELECT with the guard's reason", async () => {
  const stub = makeStubClient();
  const ctx = makeCtx(stub.client);
  const out = await dispatchTool(ctx, {
    id: "c1",
    name: "query_database",
    arguments: { sql: "DELETE FROM chat.messages WHERE 1=1" },
  });
  assert.match(out, /^Error: Only SELECT/);
  assert.equal(stub.queries.length, 0, "guard must short-circuit before running");
});

test("query_database rejects SQL referencing non-chat namespaces", async () => {
  const stub = makeStubClient();
  const ctx = makeCtx(stub.client);
  const out = await dispatchTool(ctx, {
    id: "c1",
    name: "query_database",
    arguments: { sql: "SELECT * FROM system.users" },
  });
  assert.match(out, /^Error: /);
  assert.equal(stub.queries.length, 0);
});

test("query_database surfaces upstream errors as Error: prefixed string", async () => {
  const stub = makeStubClient({ queryThrows: new Error("table doesn't exist") });
  const ctx = makeCtx(stub.client);
  const out = await dispatchTool(ctx, {
    id: "c1",
    name: "query_database",
    arguments: { sql: "SELECT * FROM chat.conversations" },
  });
  assert.match(out, /^Error: table doesn't exist/);
});

test("query_database truncates oversized result payloads", async () => {
  const bigRows = Array.from({ length: 1000 }, (_, i) => ({
    id: `${i}`,
    payload: "x".repeat(200),
  }));
  const stub = makeStubClient({ queryResult: { results: [{ named_rows: bigRows }] } });
  const ctx = makeCtx(stub.client);
  const out = await dispatchTool(ctx, {
    id: "c1",
    name: "query_database",
    arguments: { sql: "SELECT * FROM chat.messages" },
  });
  const parsed = JSON.parse(out) as { truncated: boolean; rows: unknown[] };
  assert.equal(parsed.truncated, true);
  assert.equal(parsed.rows.length, 20);
});

// =============================================================================
// delete_conversation (approval gate)
// =============================================================================

test("delete_conversation REFUSES without a prior approval", async () => {
  const stub = makeStubClient();
  const ctx = makeCtx(stub.client);
  const out = await dispatchTool(ctx, {
    id: "c1",
    name: "delete_conversation",
    arguments: { conversation_id: "11111111-1111-1111-1111-111111111111" },
  });
  assert.match(out, /requires request_approval first/);
  assert.equal(stub.queries.length, 0, "no DELETE must run without approval");
});

test("delete_conversation REFUSES if the prior approval was rejected", async () => {
  const stub = makeStubClient();
  const ctx = makeCtx(stub.client);
  ctx.lastApprovalDecision = "rejected";
  const out = await dispatchTool(ctx, {
    id: "c1",
    name: "delete_conversation",
    arguments: { conversation_id: "11111111-1111-1111-1111-111111111111" },
  });
  assert.match(out, /requires request_approval first/);
  assert.equal(stub.queries.length, 0);
});

test("delete_conversation REFUSES a non-UUID id even with approval", async () => {
  const stub = makeStubClient();
  const ctx = makeCtx(stub.client);
  ctx.lastApprovalDecision = "approved";
  const out = await dispatchTool(ctx, {
    id: "c1",
    name: "delete_conversation",
    arguments: { conversation_id: "'; DROP TABLE x;--" },
  });
  assert.match(out, /must be a UUID/);
  assert.equal(stub.queries.length, 0);
});

test("delete_conversation cascades through all child tables, then the parent", async () => {
  const stub = makeStubClient();
  const ctx = makeCtx(stub.client);
  ctx.lastApprovalDecision = "approved";
  const out = await dispatchTool(ctx, {
    id: "c1",
    name: "delete_conversation",
    arguments: { conversation_id: "11111111-1111-1111-1111-111111111111" },
  });
  assert.match(out, /^deleted conversation/);
  // 5 deletes in this exact order: typing_tokens, approvals, tasks, messages, conversations.
  assert.equal(stub.queries.length, 5);
  assert.match(stub.queries[0]!.sql, /DELETE FROM chat\.typing_tokens/);
  assert.match(stub.queries[1]!.sql, /DELETE FROM chat\.approvals/);
  assert.match(stub.queries[2]!.sql, /DELETE FROM chat\.tasks/);
  assert.match(stub.queries[3]!.sql, /DELETE FROM chat\.messages/);
  assert.match(stub.queries[4]!.sql, /DELETE FROM chat\.conversations/);
  for (const q of stub.queries) {
    assert.deepEqual(q.params, ["11111111-1111-1111-1111-111111111111"]);
  }
});

test("delete_conversation CONSUMES the approval (cannot be reused for a second delete)", async () => {
  const stub = makeStubClient();
  const ctx = makeCtx(stub.client);
  ctx.lastApprovalDecision = "approved";
  await dispatchTool(ctx, {
    id: "c1",
    name: "delete_conversation",
    arguments: { conversation_id: "11111111-1111-1111-1111-111111111111" },
  });
  // Second delete in the same turn without a fresh approval must be refused.
  stub.queries.length = 0;
  const second = await dispatchTool(ctx, {
    id: "c2",
    name: "delete_conversation",
    arguments: { conversation_id: "22222222-2222-2222-2222-222222222222" },
  });
  assert.match(second, /requires request_approval first/);
  assert.equal(stub.queries.length, 0);
});

// =============================================================================
// search_documents (RAG)
// =============================================================================

test("search_documents rejects an empty query", async () => {
  process.env.EMBEDDING_PROVIDER = "fake";
  const stub = makeStubClient();
  const ctx = makeCtx(stub.client);
  const out = await dispatchTool(ctx, {
    id: "c1",
    name: "search_documents",
    arguments: { query: "" },
  });
  assert.match(out, /^Error: query must be a non-empty string/);
});

test("search_documents embeds the query (fake) and runs COSINE_DISTANCE SELECT", async () => {
  process.env.EMBEDDING_PROVIDER = "fake";
  const stub = makeStubClient({
    queryResult: {
      results: [
        {
          named_rows: [
            { id: "doc-topics", title: "Topics", body: "...", source: "specs", distance: 0.12 },
            { id: "doc-live", title: "Live queries", body: "...", source: "specs", distance: 0.24 },
          ],
        },
      ],
    },
  });
  const ctx = makeCtx(stub.client);
  const out = await dispatchTool(ctx, {
    id: "c1",
    name: "search_documents",
    arguments: { query: "what is a topic?", limit: 2 },
  });
  const parsed = JSON.parse(out) as { row_count: number; rows: Array<{ title: string }> };
  assert.equal(parsed.row_count, 2);
  assert.equal(parsed.rows[0]!.title, "Topics");
  // Verify the SELECT shape.
  assert.equal(stub.queries.length, 1);
  assert.match(stub.queries[0]!.sql, /COSINE_DISTANCE\(embedding, '\[/);
  assert.match(stub.queries[0]!.sql, /FROM chat\.docs/);
  assert.match(stub.queries[0]!.sql, /ORDER BY distance ASC/);
  assert.match(stub.queries[0]!.sql, /LIMIT 2/);
});

test("search_documents clamps limit to [1, 10]", async () => {
  process.env.EMBEDDING_PROVIDER = "fake";
  const stub = makeStubClient();
  const ctx = makeCtx(stub.client);
  await dispatchTool(ctx, {
    id: "c1",
    name: "search_documents",
    arguments: { query: "x", limit: 9999 },
  });
  assert.match(stub.queries[0]!.sql, /LIMIT 10/);

  stub.queries.length = 0;
  await dispatchTool(ctx, {
    id: "c2",
    name: "search_documents",
    arguments: { query: "x", limit: -3 },
  });
  assert.match(stub.queries[0]!.sql, /LIMIT 1/);
});

test("search_documents surfaces upstream query errors", async () => {
  process.env.EMBEDDING_PROVIDER = "fake";
  const stub = makeStubClient({ queryThrows: new Error("docs table not found") });
  const ctx = makeCtx(stub.client);
  const out = await dispatchTool(ctx, {
    id: "c1",
    name: "search_documents",
    arguments: { query: "x" },
  });
  assert.match(out, /^Error: docs table not found/);
});

test("search_documents surfaces embedding errors when no fallback is available", async () => {
  // Force OpenAI path with no key set — embed() will reject.
  process.env.EMBEDDING_PROVIDER = "openai";
  delete process.env.OPENAI_API_KEY;
  const stub = makeStubClient();
  const ctx = makeCtx(stub.client);
  const out = await dispatchTool(ctx, {
    id: "c1",
    name: "search_documents",
    arguments: { query: "x" },
  });
  assert.match(out, /^Error: failed to embed query/);
  assert.equal(stub.queries.length, 0);
  delete process.env.EMBEDDING_PROVIDER;
});

// =============================================================================
// Unknown tool
// =============================================================================

test("dispatchTool returns 'Unknown tool: ...' for unknown names", async () => {
  const stub = makeStubClient();
  const ctx = makeCtx(stub.client);
  const out = await dispatchTool(ctx, {
    id: "c1",
    name: "set_my_password",
    arguments: {},
  });
  assert.match(out, /^Unknown tool/);
});
