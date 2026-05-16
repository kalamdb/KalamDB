import type { KalamDBClient, Unsubscribe } from "@kalamdb/client";
import type { LlmTool, LlmToolCall } from "../lib/llm/index.js";
import type { Logger } from "../lib/logger.js";
import { embed, embeddingLiteral } from "../lib/llm/embedding.js";
import { UUID_RE, uuidLit } from "./ids.js";
import { guardSelect } from "./sql-guard.js";

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
  SEARCH_DOCUMENTS_TOOL,
];

// =============================================================================
// Dispatch
// =============================================================================

export interface ToolContext {
  client: KalamDBClient;
  log: Logger;
  task: { id: string; conversation_id: string; message_id: string };
  signal: AbortSignal;
  /** Most recent tool name handled — used to enforce "request_approval first". */
  lastToolCallName: string | null;
  /** Decision returned by the most recent request_approval — 'approved' / 'rejected' / null. */
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

  const decision = await new Promise<string>((resolve) => {
    let settled = false;
    let unsubscribe: Unsubscribe | undefined;

    const finish = (value: string): void => {
      if (settled) return;
      settled = true;
      ctx.signal.removeEventListener("abort", onAbort);
      void unsubscribe?.();
      resolve(value);
    };
    const onAbort = (): void => finish("rejected (cancelled by user before approval resolved)");
    ctx.signal.addEventListener("abort", onAbort, { once: true });

    ctx.client
      .live<{ id: string; status: string }>(
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
      )
      .then((u) => {
        unsubscribe = u;
        if (settled) void u();
      })
      .catch((err) => finish(`error subscribing to approval: ${String(err)}`));
  });

  ctx.lastToolCallName = "request_approval";
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
  ctx.log.info({ sql: guard.sql }, "query_database running");
  try {
    const res = await ctx.client.query(guard.sql!);
    const rows =
      (res as { results?: Array<{ named_rows?: Array<Record<string, unknown>> }> }).results?.[0]
        ?.named_rows ?? [];
    const plainRows = rows.map((r) => {
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

  ctx.log.info({ conversation_id: id }, "delete_conversation cascade starting");
  try {
    // Cascade order matters less than completeness — children first is the
    // safe convention even when foreign keys aren't enforced.
    await ctx.client.query("DELETE FROM chat.typing_tokens WHERE conversation_id = $1", [id]);
    await ctx.client.query("DELETE FROM chat.approvals WHERE conversation_id = $1", [id]);
    await ctx.client.query("DELETE FROM chat.tasks WHERE conversation_id = $1", [id]);
    await ctx.client.query("DELETE FROM chat.messages WHERE conversation_id = $1", [id]);
    await ctx.client.query("DELETE FROM chat.conversations WHERE id = $1", [id]);
    ctx.log.info({ conversation_id: id }, "delete_conversation cascade complete");
    return `deleted conversation ${id} and all related rows`;
  } catch (err) {
    ctx.log.error({ err, conversation_id: id }, "delete_conversation failed");
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
    const rows =
      (res as { results?: Array<{ named_rows?: Array<Record<string, unknown>> }> }).results?.[0]
        ?.named_rows ?? [];
    const plain = rows.map((r) => {
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

// =============================================================================
// Internal: KalamDB cell unwrapping (shared with agent loop)
// =============================================================================

function unwrap(value: unknown): unknown {
  if (value && typeof value === "object" && "asString" in value) {
    return (value as { asString: () => string }).asString();
  }
  if (value && typeof value === "object" && "toJson" in value) {
    return (value as { toJson: () => unknown }).toJson();
  }
  return value;
}
