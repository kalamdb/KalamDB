import "dotenv/config";
import { Auth, createClient, type KalamDBClient } from "@kalamdb/client";
import {
  createConsumerClient,
  runConsumer,
  type ConsumerRunContext,
  type ConsumerChange,
} from "@kalamdb/consumer";
import {
  getLlmAdapter,
  type LlmAdapter,
  type LlmMessage,
  type LlmToolCall,
} from "../lib/llm/index.js";
import { withRetry } from "../lib/llm/retry.js";
import { withSlowdown } from "../lib/llm/slowdown.js";
import { UUID_RE, uuidLit } from "./ids.js";
import { logger } from "../lib/logger.js";
import { dispatchTool, TOOLS, type ToolContext } from "./tools.js";

const log = logger.child({ component: "agent" });

const KALAMDB_URL = process.env.KALAMDB_URL ?? "http://127.0.0.1:8080";
const USER = process.env.KALAMDB_USER ?? "root";
const PASSWORD = process.env.KALAMDB_PASSWORD ?? "kalamdb-dev-password";
const TASK_TOPIC = process.env.KALAMDB_TASK_TOPIC ?? "chat.task_events";
const GROUP_ID = process.env.KALAMDB_GROUP ?? "chat-agents";

/** Hard cap on assistant-turn iterations (text → tool calls → text → ...) per
 *  task. Generous for current tool surface but bounded so a misbehaving LLM
 *  can't loop forever. */
const MAX_TOOL_TURNS = 8;

const SYSTEM_PROMPT = `You are a concise, helpful assistant inside a KalamDB-powered chat app.

# Tools available
- request_approval(question)  — ask the user for explicit yes/no before any irreversible action.
- query_database(sql)         — run a single read-only SELECT against the chat namespace.
- delete_conversation(conversation_id) — permanently delete a conversation and its history.
- search_documents(query)     — semantic vector search over the KalamDB knowledge base for fuzzy / conceptual questions.

# Database schema (chat namespace)
Five tables; all use string UUID primary keys unless noted.

- chat.conversations(id, title, created_at, updated_at)
- chat.messages(id, conversation_id, role, body, status, created_at, updated_at)
    role:   'user' | 'assistant' | 'system'
    status: 'pending' | 'streaming' | 'final' | 'cancelled' | 'error'
- chat.typing_tokens(id, conversation_id, message_id, body, seq, created_at)
    Streaming token deltas. Cleared when the message reaches a terminal status.
- chat.approvals(id, conversation_id, message_id, question, status, created_at, resolved_at)
    status: 'pending' | 'approved' | 'rejected'
- chat.tasks(id, conversation_id, message_id, is_cancelled, started_at, finished_at)
    One row per assistant turn. finished_at = NULL while the agent is working.

# Tool-use rules
1. STRUCTURED questions about the user's OWN data (counts, listings,
   "what did I ask earlier", etc.) → use query_database. Always namespace
   tables as chat.<table>. Do NOT show the raw SQL or the JSON result to
   the user — phrase the answer in natural language. If the SELECT errors,
   retry once with a simpler query before apologizing.

   IMPORTANT: your own current in-flight assistant message is ALREADY
   inserted into chat.messages with status='streaming' before you run this
   tool. From the user's perspective that row is just a "..." typing
   indicator — they don't count it as a message. When you query
   chat.messages for counts or listings, filter to user-visible rows by
   default:
       WHERE status IN ('final','cancelled','error')
   Only include 'pending' / 'streaming' rows if the user explicitly asks
   about in-flight or unfinished messages.

2. FUZZY / CONCEPTUAL questions about KalamDB itself ("what is a topic?",
   "how does cancellation work?", "what's a live query?") → use
   search_documents. It performs vector similarity search over a curated
   knowledge base (chat.docs). Read the top results, then phrase the
   answer in natural language and CITE the document titles you used at
   the end of your reply (e.g., "(source: 'Topics & runConsumer')").
   Do NOT fabricate facts the documents don't support.

3. For any DESTRUCTIVE or IRREVERSIBLE action (delete_conversation, future
   send_email / charge / shell-command tools), you MUST call request_approval
   IMMEDIATELY BEFORE the destructive tool, in the same turn. If approval
   returns 'rejected' or 'cancelled', stop immediately and acknowledge.
   Never call delete_conversation without an approval that just returned
   'approved'.

4. Do not use request_approval for cosmetic confirmations or trivial
   clarifications — ask in plain text for those.

5. Keep replies short unless the user asks for detail. Don't narrate your
   tool use ("Let me check the database..."); just produce the result.`;

interface Task {
  id: string;
  conversation_id: string;
  message_id: string;
}

const activeControllers = new Map<string, AbortController>();

async function main(): Promise<void> {
  // Two clients: the consumer client drives runConsumer (topic pulls + acks),
  // and a regular KalamDB client handles SQL + live queries (per-task cancel
  // subscriptions + approval-resolution subscriptions).
  const consumerClient = createConsumerClient({
    url: KALAMDB_URL,
    authProvider: async () => Auth.basic(USER, PASSWORD),
    disableCompression: true,
  });
  const sqlClient = createClient({
    url: KALAMDB_URL,
    authProvider: async () => Auth.basic(USER, PASSWORD),
    disableCompression: true,
  });
  await sqlClient.connect();
  let llm = withRetry(await getLlmAdapter(), undefined, log);
  // Recorder-only knob: when set, wraps the adapter with a per-token sleep so
  // the demo recorder has a window to click Stop before fast models finish.
  // Has no effect in production deployments (env is unset).
  const slowdownMs = Number(process.env.RECORDER_SLOWDOWN_MS ?? "0");
  if (slowdownMs > 0) {
    llm = withSlowdown(llm, slowdownMs);
    log.warn({ slowdown_ms: slowdownMs }, "recorder slowdown active — DO NOT USE IN PRODUCTION");
  }
  log.info({ url: KALAMDB_URL, llm: llm.name, topic: TASK_TOPIC, group: GROUP_ID }, "agent ready");

  process.on("unhandledRejection", (reason) => {
    log.error({ err: reason }, "unhandled rejection");
  });

  const stop = new AbortController();
  const shutdown = async (signal: string): Promise<void> => {
    log.info({ signal }, "shutting down");
    stop.abort();
    for (const ctrl of activeControllers.values()) ctrl.abort();
    await sqlClient.disconnect().catch(() => undefined);
    process.exit(0);
  };
  process.on("SIGINT", () => void shutdown("SIGINT"));
  process.on("SIGTERM", () => void shutdown("SIGTERM"));

  // Work queue. The topic is sourced from chat.tasks ON INSERT, so each new
  // task arrives here with at-least-once delivery and group-based load
  // balancing across replicas of this process.
  await runConsumer<Record<string, unknown>>({
    client: consumerClient,
    name: "chat-agent",
    topic: TASK_TOPIC,
    groupId: GROUP_ID,
    start: "earliest",
    batchSize: 10,
    timeoutSeconds: 30,
    stopSignal: stop.signal,
    onChange: async (
      _ctx: ConsumerRunContext<Record<string, unknown>>,
      change: ConsumerChange<Record<string, unknown>>,
    ) => {
      const row = change.data;
      const task: Task = {
        id: String(unwrap(row.id) ?? ""),
        conversation_id: String(unwrap(row.conversation_id) ?? ""),
        message_id: String(unwrap(row.message_id) ?? ""),
      };
      if (
        !UUID_RE.test(task.id) ||
        !UUID_RE.test(task.conversation_id) ||
        !UUID_RE.test(task.message_id)
      ) {
        log.warn({ row }, "dropping malformed task event");
        return;
      }
      if (activeControllers.has(task.id)) return;

      // At-least-once redelivery: skip tasks already finished or cancelled.
      // isTaskTerminal returns null when the row hasn't become readable yet
      // (rare topic-vs-table read-visibility race) — treat that as "not
      // terminal, proceed", so the worker still picks the work up.
      const terminal = await isTaskTerminal(sqlClient, task.id, log);
      if (terminal === true) return;

      const ctrl = new AbortController();
      activeControllers.set(task.id, ctrl);
      try {
        await runTask(sqlClient, llm, task, ctrl);
      } finally {
        activeControllers.delete(task.id);
      }
    },
    onConnectionRetry: ({ attempt, backoffMs, error }) => {
      log.warn({ attempt, backoffMs, err: error }, "consumer reconnecting");
    },
    onConnectionRestored: ({ attempt }) => {
      log.info({ attempt }, "consumer reconnected");
    },
    onConnectionError: ({ error, attempt }) => {
      log.error({ attempt, err: error }, "consumer giving up");
    },
  });
}

async function runTask(
  client: KalamDBClient,
  llm: LlmAdapter,
  task: Task,
  controller: AbortController,
): Promise<void> {
  const tlog = log.child({
    task_id: task.id,
    message_id: task.message_id,
    conversation_id: task.conversation_id,
  });
  tlog.info("task started");

  // Per-task cancellation subscription, scoped to this row only — replaces
  // the previous global live-on-chat.tasks fan-out.
  const cancelUnsub = await client.live<{ id: string; is_cancelled: boolean | string }>(
    `SELECT id, is_cancelled FROM chat.tasks WHERE id = ${uuidLit(task.id)}`,
    (rows) => {
      for (const row of rows) {
        const cancelled = unwrap(row.is_cancelled);
        if ((cancelled === true || cancelled === "true") && !controller.signal.aborted) {
          tlog.info("cancel signal received");
          controller.abort();
        }
      }
    },
  );

  // Startup race: between isTaskTerminal returning false and the live sub
  // attaching, the user could click Stop. Re-read the row now that we're
  // subscribed and bail before doing any writes.
  const stillTerminal = await isTaskTerminal(client, task.id, tlog);
  if (stillTerminal === true) {
    tlog.info("task became terminal during subscription attach — bailing");
    await cancelUnsub().catch(() => undefined);
    return;
  }

  const assembled: string[] = [];
  let buffer = "";
  let flushTimer: NodeJS.Timeout | null = null;
  let seq = 0;
  let finalStatus: "final" | "cancelled" | "error" = "error";

  const cleanup = async (): Promise<void> => {
    if (flushTimer) {
      clearTimeout(flushTimer);
      flushTimer = null;
    }
    buffer = "";
    await cancelUnsub().catch(() => undefined);
    await clearTypingTokens(client, task.message_id).catch(() => undefined);
    await finalizeTask(client, task.id).catch(() => undefined);
    tlog.info({ final_status: finalStatus }, "task complete");
  };

  try {
    await ensureAssistantStub(client, task);
    await markStreaming(client, task.message_id);

    // The LLM needs to know which conversation it's in so delete_conversation,
    // query_database etc. can scope to the right id. Inject it as a separate
    // system message so the static SYSTEM_PROMPT stays cache-friendly.
    const baseMessages: LlmMessage[] = [
      { role: "system", content: SYSTEM_PROMPT },
      {
        role: "system",
        content: `# Current conversation\nconversation_id = ${task.conversation_id}`,
      },
    ];
    baseMessages.push(...(await fetchHistory(client, task.conversation_id)));
    tlog.debug({ history: baseMessages.length - 2 }, "starting LLM turn");

    const flush = async () => {
      if (flushTimer) {
        clearTimeout(flushTimer);
        flushTimer = null;
      }
      if (!buffer) return;
      const body = buffer;
      buffer = "";
      try {
        await client.insert("chat.typing_tokens", {
          id: crypto.randomUUID(),
          conversation_id: task.conversation_id,
          message_id: task.message_id,
          body,
          seq: ++seq,
          created_at: new Date().toISOString(),
        });
      } catch (e) {
        tlog.error({ err: e }, "typing-token flush failed");
      }
    };
    const enqueue = (delta: string) => {
      buffer += delta;
      if (buffer.length >= 16) {
        void flush();
      } else if (!flushTimer) {
        flushTimer = setTimeout(() => {
          void flush();
        }, 200);
      }
    };

    const messages: LlmMessage[] = [...baseMessages];
    const toolCtx: ToolContext = {
      client,
      log: tlog,
      task,
      signal: controller.signal,
      lastApprovalDecision: null,
    };
    let turn = 0;
    while (turn < MAX_TOOL_TURNS) {
      turn++;
      const pendingCalls: LlmToolCall[] = [];
      let stopReason: "stop" | "tool_calls" | "length" | "error" = "stop";

      try {
        for await (const event of llm.stream({
          messages,
          tools: TOOLS,
          signal: controller.signal,
        })) {
          if (event.type === "text") {
            assembled.push(event.delta);
            enqueue(event.delta);
          } else if (event.type === "tool_call") {
            await flush();
            pendingCalls.push(event.call);
          } else if (event.type === "done") {
            stopReason = event.reason;
          }
        }
      } catch (error) {
        if (controller.signal.aborted) {
          finalStatus = "cancelled";
          await finalizeMessage(client, task.message_id, assembled.join(""), "cancelled");
          return;
        }
        tlog.error({ err: error }, "llm error");
        finalStatus = "error";
        await finalizeMessage(client, task.message_id, "Sorry — something went wrong.", "error");
        return;
      }

      if (controller.signal.aborted) {
        finalStatus = "cancelled";
        await finalizeMessage(client, task.message_id, assembled.join(""), "cancelled");
        return;
      }

      if (stopReason !== "tool_calls" || pendingCalls.length === 0) {
        await flush();
        finalStatus = "final";
        await finalizeMessage(client, task.message_id, assembled.join(""), "final");
        return;
      }

      messages.push({
        role: "assistant",
        content: assembled.join(""),
        toolCalls: pendingCalls,
      });

      for (const call of pendingCalls) {
        const result = await dispatchTool(toolCtx, call);
        messages.push({
          role: "tool",
          toolCallId: call.id,
          content: result,
        });
        if (controller.signal.aborted) {
          finalStatus = "cancelled";
          await finalizeMessage(client, task.message_id, assembled.join(""), "cancelled");
          return;
        }
      }
    }

    finalStatus = "final";
    await finalizeMessage(
      client,
      task.message_id,
      assembled.join("") + "\n\n(Agent exceeded tool-call turn limit.)",
      "final",
    );
  } finally {
    await cleanup();
  }
}

async function fetchHistory(client: KalamDBClient, conversationId: string): Promise<LlmMessage[]> {
  const res = await client.query(
    `SELECT role, body FROM chat.messages
       WHERE conversation_id = $1
         AND status IN ('final', 'streaming')
       ORDER BY created_at ASC`,
    [conversationId],
  );
  const rows =
    (res as { results?: Array<{ named_rows?: Array<Record<string, unknown>> }> }).results?.[0]
      ?.named_rows ?? [];
  const out: LlmMessage[] = [];
  for (const row of rows) {
    const role = unwrap(row.role);
    const body = unwrap(row.body);
    if (role !== "user" && role !== "assistant") continue;
    if (typeof body !== "string" || body.length === 0) continue;
    out.push({ role, content: body });
  }
  return out;
}

/**
 * Returns:
 *   - true  → task is finished or cancelled (skip).
 *   - false → task is active, proceed.
 *   - null  → row doesn't exist yet (topic event preceded table read-
 *             visibility). Caller should proceed; the row will appear soon.
 */
async function isTaskTerminal(
  client: KalamDBClient,
  taskId: string,
  log_: typeof log,
): Promise<boolean | null> {
  const res = await client.query(`SELECT finished_at, is_cancelled FROM chat.tasks WHERE id = $1`, [
    taskId,
  ]);
  const row = (res as { results?: Array<{ named_rows?: Array<Record<string, unknown>> }> })
    .results?.[0]?.named_rows?.[0];
  if (!row) {
    log_.warn({ task_id: taskId }, "task row not yet visible after topic event — proceeding");
    return null;
  }
  const finishedAt = unwrap(row.finished_at);
  const cancelled = unwrap(row.is_cancelled);
  return Boolean(finishedAt) || cancelled === true || cancelled === "true";
}

async function ensureAssistantStub(client: KalamDBClient, task: Task): Promise<void> {
  // Two-tier idempotency:
  //   1. Cheap: SELECT-then-skip handles the common redelivery case.
  //   2. Safe: PK collision on INSERT handles the rebalance race where two
  //      consumers in the same group transiently both see the event. We
  //      catch any "duplicate key / already exists" error from KalamDB and
  //      treat it as success; anything else propagates.
  const res = await client.query(`SELECT id FROM chat.messages WHERE id = $1`, [task.message_id]);
  const exists =
    ((res as { results?: Array<{ named_rows?: Array<unknown> }> }).results?.[0]?.named_rows
      ?.length ?? 0) > 0;
  if (exists) return;
  const now = new Date().toISOString();
  try {
    await client.insert("chat.messages", {
      id: task.message_id,
      conversation_id: task.conversation_id,
      role: "assistant",
      body: "",
      status: "pending",
      created_at: now,
      updated_at: now,
    });
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    if (/duplicate|already exists|unique|primary key/i.test(msg)) {
      log.warn({ message_id: task.message_id }, "assistant stub already exists (race) — ignored");
      return;
    }
    throw err;
  }
}

async function markStreaming(client: KalamDBClient, messageId: string): Promise<void> {
  await client.update("chat.messages", messageId, {
    status: "streaming",
    updated_at: new Date().toISOString(),
  });
}

async function finalizeMessage(
  client: KalamDBClient,
  messageId: string,
  body: string,
  status: "final" | "cancelled" | "error",
): Promise<void> {
  await client.update("chat.messages", messageId, {
    body,
    status,
    updated_at: new Date().toISOString(),
  });
}

async function finalizeTask(client: KalamDBClient, taskId: string): Promise<void> {
  await client.update("chat.tasks", taskId, {
    finished_at: new Date().toISOString(),
  });
}

async function clearTypingTokens(client: KalamDBClient, messageId: string): Promise<void> {
  const res = await client.query(`SELECT id FROM chat.typing_tokens WHERE message_id = $1`, [
    messageId,
  ]);
  const rows =
    (res as { results?: Array<{ named_rows?: Array<Record<string, unknown>> }> }).results?.[0]
      ?.named_rows ?? [];
  for (const row of rows) {
    const id = unwrap(row.id);
    if (typeof id === "string" && UUID_RE.test(id)) {
      await client.delete("chat.typing_tokens", id);
    }
  }
}

function unwrap(value: unknown): any {
  if (value && typeof value === "object" && "asString" in value) {
    return (value as { asString: () => string }).asString();
  }
  if (value && typeof value === "object" && "toJson" in value) {
    return (value as { toJson: () => unknown }).toJson();
  }
  return value;
}

main().catch((err) => {
  log.fatal({ err }, "agent fatal");
  process.exit(1);
});
