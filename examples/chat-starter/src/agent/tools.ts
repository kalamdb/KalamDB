import type { KalamDBClient, Unsubscribe } from "@kalamdb/client";
import type { LlmTool, LlmToolCall } from "../lib/llm/index.js";
import type { Logger } from "../lib/logger.js";
import { embed, embeddingLiteral } from "../lib/llm/embedding.js";
import { UUID_RE, uuidLit } from "./ids.js";
import { guardSelect } from "./sql-guard.js";
import { extractRows, kdbBigIntToNumber, unwrap } from "../lib/kdb-row.js";

// =============================================================================
// Tool definitions sent to the LLM.
// =============================================================================

export const REQUEST_APPROVAL_TOOL: LlmTool = {
  name: "request_approval",
  description:
    "Pause and request explicit user approval before performing a risky or irreversible action. Returns 'approved' or 'rejected'.",
  parameters: {
    type: "object",
    properties: {
      question: {
        type: "string",
        description:
          "A clear, specific yes/no question describing exactly what you are about to do. The user sees this verbatim.",
      },
    },
    required: ["question"],
    additionalProperties: false,
  },
};

export const QUERY_DATABASE_TOOL: LlmTool = {
  name: "query_database",
  description:
    "Run a single read-only SELECT against the chat namespace. Use this whenever the user asks a question that can be answered from their data (counts, listings, lookups, history searches). The result comes back as a JSON array of rows; phrase the final answer to the user in natural language.",
  parameters: {
    type: "object",
    properties: {
      sql: {
        type: "string",
        description:
          "A single SELECT statement. Must reference tables as chat.<table>. Comments and multiple statements are rejected. If you omit LIMIT, one will be added automatically (default 200).",
      },
    },
    required: ["sql"],
    additionalProperties: false,
  },
};

export const DELETE_CONVERSATION_TOOL: LlmTool = {
  name: "delete_conversation",
  description:
    "Permanently delete a conversation and all of its messages, typing tokens, approvals, and tasks. Destructive and irreversible — you MUST call request_approval immediately before this tool, in the same turn, and only proceed if it returned 'approved'.",
  parameters: {
    type: "object",
    properties: {
      conversation_id: {
        type: "string",
        description: "The UUID of the conversation to delete.",
      },
    },
    required: ["conversation_id"],
    additionalProperties: false,
  },
};

export const DELETE_ALL_CONVERSATIONS_TOOL: LlmTool = {
  name: "delete_all_conversations",
  description:
    "Permanently delete EVERY conversation owned by the current user, along with all messages, typing tokens, approvals, and tasks. Destructive and irreversible — you MUST call request_approval immediately before this tool, in the same turn, with a question that makes the bulk scope explicit (e.g. 'Permanently delete all N of your conversations?'), and only proceed if it returned 'approved'. Use this instead of looping delete_conversation when the user asks for 'delete all', 'wipe everything', 'clear my history', etc.",
  parameters: {
    type: "object",
    properties: {},
    required: [],
    additionalProperties: false,
  },
};

export const SEARCH_DOCUMENTS_TOOL: LlmTool = {
  name: "search_documents",
  description:
    "Semantic search over the KalamDB knowledge base (chat.docs) using vector similarity. Use this for FUZZY / CONCEPTUAL questions about KalamDB itself ('what is a topic?', 'how does cancellation work?', 'how do I use live queries?'). Returns the top matching documents as a JSON array with title, body, source, and distance. Always cite the doc titles you used when phrasing the answer.",
  parameters: {
    type: "object",
    properties: {
      query: {
        type: "string",
        description: "The natural-language query to embed and search for.",
      },
      limit: {
        type: "integer",
        description: "Maximum number of documents to return. Default 5, max 10.",
      },
    },
    required: ["query"],
    additionalProperties: false,
  },
};

export const TOOLS: LlmTool[] = [
  REQUEST_APPROVAL_TOOL,
  QUERY_DATABASE_TOOL,
  DELETE_CONVERSATION_TOOL,
  DELETE_ALL_CONVERSATIONS_TOOL,
  SEARCH_DOCUMENTS_TOOL,
];

// =============================================================================
// Dispatch
// =============================================================================

export interface ToolContext {
  /** KalamDB client authenticated AS the task owner. All SQL the tools
   *  issue is automatically scoped to that user's partition — no per-call
   *  EXECUTE AS USER wrapping needed (and the subscription endpoint
   *  wouldn't accept it anyway). */
  client: KalamDBClient;
  log: Logger;
  task: { id: string; conversation_id: string; message_id: string; user: string };
  signal: AbortSignal;
  /** Decision returned by the most recent request_approval — 'approved' / 'rejected' / null.
   *  Consumed (set to null) by destructive tools so a single approval can't
   *  authorize two destructive actions. */
  lastApprovalDecision: string | null;
}

export async function dispatchTool(ctx: ToolContext, call: LlmToolCall): Promise<string> {
  switch (call.name) {
    case "request_approval":
      return await handleRequestApproval(ctx, call);
    case "query_database":
      return await handleQueryDatabase(ctx, call);
    case "delete_conversation":
      return await handleDeleteConversation(ctx, call);
    case "delete_all_conversations":
      return await handleDeleteAllConversations(ctx);
    case "search_documents":
      return await handleSearchDocuments(ctx, call);
    default:
      return `Unknown tool: ${call.name}`;
  }
}

// =============================================================================
// request_approval
// =============================================================================

async function handleRequestApproval(ctx: ToolContext, call: LlmToolCall): Promise<string> {
  const question =
    typeof call.arguments.question === "string"
      ? call.arguments.question
      : "The assistant is requesting approval.";
  const approvalId = crypto.randomUUID();

  await ctx.client.insert("chat.approvals", {
    id: approvalId,
    conversation_id: ctx.task.conversation_id,
    message_id: ctx.task.message_id,
    question,
    status: "pending",
    created_at: new Date().toISOString(),
    resolved_at: null,
  });
  ctx.log.info({ approval_id: approvalId, question }, "approval pending");

  // Wire up the resolver state machine BEFORE subscribing — earlier this
  // lived inside a .then() chain on the same Promise that resolved the
  // decision, which left a window where `unsubscribe` was still undefined
  // and an early abort could leak the subscription. ctx.client is
  // authenticated AS the task owner, so the SELECT runs in the owner's
  // partition without any EXECUTE AS USER wrapping.
  let settled = false;
  let unsubscribe: Unsubscribe | null = null;
  let resolveDecision!: (value: string) => void;
  const decisionPromise = new Promise<string>((resolve) => {
    resolveDecision = resolve;
  });
  const finish = (value: string): void => {
    if (settled) return;
    settled = true;
    ctx.signal.removeEventListener("abort", onAbort);
    if (unsubscribe) void unsubscribe();
    resolveDecision(value);
  };
  const onAbort = (): void => {
    ctx.log.info({ approval_id: approvalId }, "approval rejected by abort signal");
    finish("rejected");
  };
  if (ctx.signal.aborted) {
    return "rejected";
  }
  ctx.signal.addEventListener("abort", onAbort, { once: true });

  try {
    unsubscribe = await ctx.client.live<{ id: string; status: string }>(
      `SELECT id, status FROM chat.approvals WHERE id = ${uuidLit(approvalId)}`,
      (rows) => {
        const row = rows[0];
        if (!row) return;
        const status = unwrap(row.status);
        if (status === "approved" || status === "rejected") {
          ctx.log.info({ approval_id: approvalId, status }, "approval resolved");
          finish(status);
        }
      },
      {
        onError: (err) => {
          ctx.log.error({ approval_id: approvalId, err }, "approval live errored");
          finish(`error subscribing to approval: ${String(err)}`);
        },
      },
    );
    // If the abort listener fired *during* the await above, settled is
    // already true and the unsubscribe handle we just received needs
    // teardown now (finish() ran before unsubscribe was assigned).
    if (settled) void unsubscribe();
  } catch (err) {
    finish(`error subscribing to approval: ${String(err)}`);
  }

  const decision = await decisionPromise;

  ctx.lastApprovalDecision = decision;
  return decision;
}

// =============================================================================
// query_database
// =============================================================================

const MAX_QUERY_RESULT_BYTES = 16 * 1024;

async function handleQueryDatabase(ctx: ToolContext, call: LlmToolCall): Promise<string> {
  const raw = typeof call.arguments.sql === "string" ? call.arguments.sql : "";
  const guard = guardSelect(raw);
  if (!guard.ok) {
    ctx.log.warn({ sql: raw, reason: guard.reason }, "query_database rejected by guard");
    return `Error: ${guard.reason}`;
  }
  ctx.log.info({ sql: guard.sql, user: ctx.task.user }, "query_database running");
  try {
    // ctx.client is authenticated AS the task owner; KalamDB's USER table
    // partitioning serves rows from that user's partition only. Even if the
    // LLM tries to inspect another user's data, the multi-tenant fence holds.
    const res = await ctx.client.query(guard.sql!);
    const plainRows = extractRows(res).map((r) => {
      const out: Record<string, unknown> = {};
      for (const [k, v] of Object.entries(r)) out[k] = unwrap(v);
      return out;
    });
    const serialized = JSON.stringify({ row_count: plainRows.length, rows: plainRows });
    if (serialized.length > MAX_QUERY_RESULT_BYTES) {
      return JSON.stringify({
        row_count: plainRows.length,
        truncated: true,
        rows: plainRows.slice(0, 20),
        note: "Result truncated to first 20 rows — refine the query (SELECT specific columns, narrower WHERE, smaller LIMIT).",
      });
    }
    return serialized;
  } catch (err) {
    ctx.log.error({ err }, "query_database failed");
    return `Error: ${err instanceof Error ? err.message : String(err)}`;
  }
}

// =============================================================================
// delete_conversation
// =============================================================================

async function handleDeleteConversation(ctx: ToolContext, call: LlmToolCall): Promise<string> {
  if (ctx.lastApprovalDecision !== "approved") {
    return "Error: delete_conversation requires request_approval first with an 'approved' decision in the same turn.";
  }
  // One-shot consume — prevents reusing an approval for a second delete.
  ctx.lastApprovalDecision = null;

  const id =
    typeof call.arguments.conversation_id === "string" ? call.arguments.conversation_id : "";
  if (!UUID_RE.test(id)) {
    return `Error: conversation_id must be a UUID; got "${id}".`;
  }

  // Honor cancellation: if the user clicked Stop between the approval and
  // here, don't proceed with destructive work.
  if (ctx.signal.aborted) {
    return "Error: cancelled before delete could start";
  }

  ctx.log.info(
    { conversation_id: id, user: ctx.task.user },
    "delete_conversation cascade starting",
  );
  try {
    // Atomic cascade: KalamDB executes BEGIN/COMMIT in a single SQL request
    // as one transaction (any DELETE failure → automatic rollback → no
    // half-deleted state). ctx.client is authenticated AS the task owner,
    // so the entire transaction runs in their partition — even if the LLM
    // pasted in a conversation_id belonging to another user, KalamDB simply
    // wouldn't find the row.
    const idLit = uuidLit(id);
    const sql = [
      "BEGIN",
      `DELETE FROM chat.typing_tokens WHERE conversation_id = ${idLit}`,
      `DELETE FROM chat.approvals WHERE conversation_id = ${idLit}`,
      `DELETE FROM chat.tasks WHERE conversation_id = ${idLit}`,
      `DELETE FROM chat.messages WHERE conversation_id = ${idLit}`,
      `DELETE FROM chat.conversations WHERE id = ${idLit}`,
      "COMMIT",
    ].join("; ");
    await ctx.client.query(sql);
    ctx.log.info({ conversation_id: id }, "delete_conversation cascade complete");
    return `deleted conversation ${id} and all related rows`;
  } catch (err) {
    ctx.log.error({ err, conversation_id: id }, "delete_conversation failed (rolled back)");
    return `Error: ${err instanceof Error ? err.message : String(err)}`;
  }
}

// =============================================================================
// delete_all_conversations
// =============================================================================

async function handleDeleteAllConversations(ctx: ToolContext): Promise<string> {
  if (ctx.lastApprovalDecision !== "approved") {
    return "Error: delete_all_conversations requires request_approval first with an 'approved' decision in the same turn.";
  }
  // One-shot consume — the LLM can't reuse this approval for a second
  // destructive call.
  ctx.lastApprovalDecision = null;

  if (ctx.signal.aborted) {
    return "Error: cancelled before delete could start";
  }

  ctx.log.info({ user: ctx.task.user }, "delete_all_conversations starting");
  try {
    // The user's just-sent message, the in-flight task row, and the
    // assistant stub for THIS turn were all written before this tool
    // dispatched — any time-based filter ("rows older than now") would
    // nuke them too and the agent would crash trying to finalize rows
    // that no longer exist. Scope the cascade explicitly by excluding
    // the three IDs we own for the current turn.
    const currentConv = uuidLit(ctx.task.conversation_id);
    const currentTask = uuidLit(ctx.task.id);
    const currentMsg = uuidLit(ctx.task.message_id);

    const countRes = await ctx.client.query(
      `SELECT count(*) AS n FROM chat.conversations WHERE id != ${currentConv}`,
    );
    const n = kdbBigIntToNumber(extractRows(countRes)[0]?.n);
    if (n === 0) {
      ctx.log.info({ user: ctx.task.user }, "delete_all_conversations: nothing to delete");
      return "deleted 0 conversations (the current one is excluded; there were no others)";
    }

    // Atomic cascade. Order matters — child rows first, parent last —
    // so a partial failure can't leave dangling FK-shaped references
    // (KalamDB doesn't enforce FKs but the UI joins on these so leaving
    // orphans would surface as ghost rows). The current turn's task /
    // message / conversation are explicitly excluded so the in-flight
    // agent state survives.
    const sql = [
      "BEGIN",
      `DELETE FROM chat.typing_tokens WHERE conversation_id != ${currentConv}`,
      `DELETE FROM chat.approvals WHERE conversation_id != ${currentConv}`,
      `DELETE FROM chat.tasks WHERE id != ${currentTask}`,
      `DELETE FROM chat.messages WHERE id != ${currentMsg}`,
      `DELETE FROM chat.conversations WHERE id != ${currentConv}`,
      "COMMIT",
    ].join("; ");
    await ctx.client.query(sql);
    ctx.log.info({ user: ctx.task.user, n }, "delete_all_conversations complete");
    return `deleted ${n} other conversations and all related rows (the one you're chatting in was kept)`;
  } catch (err) {
    ctx.log.error({ err, user: ctx.task.user }, "delete_all_conversations failed (rolled back)");
    return `Error: ${err instanceof Error ? err.message : String(err)}`;
  }
}

// =============================================================================
// search_documents (RAG)
// =============================================================================

const MAX_SEARCH_LIMIT = 10;
const DEFAULT_SEARCH_LIMIT = 5;
const MAX_SEARCH_RESULT_BYTES = 16 * 1024;

async function handleSearchDocuments(ctx: ToolContext, call: LlmToolCall): Promise<string> {
  const query = typeof call.arguments.query === "string" ? call.arguments.query.trim() : "";
  if (!query) {
    return "Error: query must be a non-empty string.";
  }
  let limit = DEFAULT_SEARCH_LIMIT;
  if (typeof call.arguments.limit === "number" && Number.isFinite(call.arguments.limit)) {
    limit = Math.max(1, Math.min(MAX_SEARCH_LIMIT, Math.floor(call.arguments.limit)));
  }
  ctx.log.info({ query, limit }, "search_documents running");

  let vec: number[];
  try {
    vec = await embed(query);
  } catch (err) {
    ctx.log.error({ err }, "search_documents embedding failed");
    return `Error: failed to embed query: ${err instanceof Error ? err.message : String(err)}`;
  }
  const vecLit = embeddingLiteral(vec);

  try {
    const res = await ctx.client.query(
      `SELECT id, title, body, source,
              COSINE_DISTANCE(embedding, '${vecLit}') AS distance
       FROM chat.docs
       ORDER BY distance ASC
       LIMIT ${limit}`,
    );
    const plain = extractRows(res).map((r) => {
      const out: Record<string, unknown> = {};
      for (const [k, v] of Object.entries(r)) out[k] = unwrap(v);
      return out;
    });
    const serialized = JSON.stringify({ row_count: plain.length, rows: plain });
    if (serialized.length > MAX_SEARCH_RESULT_BYTES) {
      // Truncate body fields to a preview if the payload is too large.
      const trimmed = plain.map((row) => {
        const body = typeof row.body === "string" ? row.body : String(row.body ?? "");
        return { ...row, body: body.length > 400 ? body.slice(0, 400) + "…" : body };
      });
      return JSON.stringify({
        row_count: trimmed.length,
        truncated: true,
        rows: trimmed,
        note: "Document bodies were truncated to ~400 chars each because the full payload exceeded the response cap.",
      });
    }
    return serialized;
  } catch (err) {
    ctx.log.error({ err }, "search_documents failed");
    return `Error: ${err instanceof Error ? err.message : String(err)}`;
  }
}
