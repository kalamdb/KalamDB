import type { KalamDBClient } from "@kalamdb/client";
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

/** In-flight request_approval waiters, keyed by approval id. The agent's
 *  approval-resolution consumer (see agent/index.ts) calls the registered
 *  resolver when the matching chat.approvals row's status flips. */
export type PendingApprovals = Map<string, (status: string) => void>;

export interface ToolContext {
  /** SQL client logged in as the agent's admin identity. Every USER-table
   *  DML / query in this module is wrapped via
   *  `client.executeAsUser(sql, ctx.task.user, params)` so KalamDB applies
   *  the multi-tenant fence server-side. SHARED tables (chat.docs) are
   *  queried directly. */
  client: KalamDBClient;
  log: Logger;
  task: { id: string; conversation_id: string; message_id: string; user: string };
  signal: AbortSignal;
  /** Decision returned by the most recent request_approval — 'approved' / 'rejected' / null.
   *  Consumed (set to null) by destructive tools so a single approval can't
   *  authorize two destructive actions. */
  lastApprovalDecision: string | null;
  /** Shared waiter registry written into by request_approval and drained by
   *  the chat.approval_resolutions consumer in agent/index.ts. */
  pendingApprovals: PendingApprovals;
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

  // If the task was already aborted before we even got here, short-circuit
  // before writing to the DB (the resolver never gets registered, so the
  // approval-resolution consumer would never wake us).
  if (ctx.signal.aborted) return "rejected";

  await ctx.client.executeAsUser(
    `INSERT INTO chat.approvals (id, conversation_id, message_id, question, status, created_at, resolved_at)
     VALUES ($1, $2, $3, $4, $5, $6, $7)`,
    ctx.task.user,
    [
      approvalId,
      ctx.task.conversation_id,
      ctx.task.message_id,
      question,
      "pending",
      new Date().toISOString(),
      null,
    ],
  );
  ctx.log.info({ approval_id: approvalId, question }, "approval pending");

  // Register a waiter in the shared map. The chat.approval_resolutions
  // consumer (one per agent process, see agent/index.ts) calls our
  // resolver when the matching approval row flips to approved / rejected.
  //
  // No per-approval live() subscription anymore — that's the whole point
  // of the topic refactor: O(active-tasks) live subscriptions collapse to
  // O(1) consumer per process.
  let settled = false;
  let resolveDecision!: (value: string) => void;
  const decisionPromise = new Promise<string>((resolve) => {
    resolveDecision = resolve;
  });
  const finish = (value: string): void => {
    if (settled) return;
    settled = true;
    ctx.signal.removeEventListener("abort", onAbort);
    ctx.pendingApprovals.delete(approvalId);
    resolveDecision(value);
  };
  const onAbort = (): void => {
    ctx.log.info({ approval_id: approvalId }, "approval rejected by abort signal");
    finish("rejected");
  };
  ctx.signal.addEventListener("abort", onAbort, { once: true });
  ctx.pendingApprovals.set(approvalId, finish);

  // Bounded wait. If the UI never resolves the approval (user closed
  // the tab, browser crashed, network died), the runConsumer slot would
  // be held forever and back up the entire task topic. Treat a five-
  // minute hang as a timeout-rejection so the agent finalizes and the
  // consumer can commit.
  const APPROVAL_TIMEOUT_MS = 5 * 60 * 1000;
  const timeout = setTimeout(() => {
    ctx.log.warn({ approval_id: approvalId }, "approval timed out — treating as rejected");
    finish("rejected");
  }, APPROVAL_TIMEOUT_MS);
  try {
    const decision = await decisionPromise;
    ctx.lastApprovalDecision = decision;
    return decision;
  } finally {
    clearTimeout(timeout);
  }
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
    // The agent's client is the admin; executeAsUser pins the SELECT to
    // the task owner's partition. Even if the LLM tries to inspect
    // another user's data, KalamDB's USER-table partitioning serves rows
    // from ctx.task.user's partition only.
    const res = await ctx.client.executeAsUser(guard.sql!, ctx.task.user);
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
    // Atomic cascade wrapped in one EXECUTE AS USER (BEGIN; ...; COMMIT;).
    // KalamDB executes BEGIN/COMMIT in a single SQL request as one
    // transaction — any DELETE failure → automatic rollback → no half-
    // deleted state. The wrap pins every statement to the task owner's
    // partition.
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
    await ctx.client.executeAsUser(sql, ctx.task.user);
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

    const countRes = await ctx.client.executeAsUser(
      `SELECT count(*) AS n FROM chat.conversations WHERE id != ${currentConv}`,
      ctx.task.user,
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
    await ctx.client.executeAsUser(sql, ctx.task.user);
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
    // chat.docs is SHARED with ACCESS LEVEL PUBLIC — every user can read it,
    // so we don't need (and can't usefully use) executeAsUser here.
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
