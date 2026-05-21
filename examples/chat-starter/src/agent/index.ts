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
import { UUID_RE } from "./ids.js";
import { logger } from "../lib/logger.js";
import { USER_RE } from "../lib/user.js";
import { extractRows, unwrap } from "../lib/kdb-row.js";
import { dispatchTool, TOOLS, type ToolContext, type PendingApprovals } from "./tools.js";

const log = logger.child({ component: "agent" });

const KALAMDB_URL = process.env.KALAMDB_URL ?? "http://127.0.0.1:2900";
const ADMIN_USER = process.env.KALAMDB_USER ?? "root";
const ADMIN_PASSWORD = process.env.KALAMDB_PASSWORD ?? "kalamdb-dev-password";
const TASK_TOPIC = process.env.KALAMDB_TASK_TOPIC ?? "chat.task_events";
const CANCEL_TOPIC = process.env.KALAMDB_CANCEL_TOPIC ?? "chat.task_cancels";
const APPROVAL_TOPIC = process.env.KALAMDB_APPROVAL_TOPIC ?? "chat.approval_resolutions";
const GROUP_ID = process.env.KALAMDB_GROUP ?? "chat-agents";

/** Hard cap on assistant-turn iterations (text → tool calls → text → ...) per
 *  task. Generous for current tool surface but bounded so a misbehaving LLM
 *  can't loop forever. */
const MAX_TOOL_TURNS = 8;

const SYSTEM_PROMPT = `You are a concise, helpful assistant inside a KalamDB-powered chat app.

# Tools available
- request_approval(question)  — ask the user for explicit yes/no before any irreversible action.
- query_database(sql)         — run a single read-only SELECT against the chat namespace.
- delete_conversation(conversation_id) — permanently delete ONE conversation and its history.
- delete_all_conversations()  — permanently delete EVERY conversation the user owns. Use this for "delete all", "wipe everything", "clear my history".
- search_documents(query)     — semantic vector search over the KalamDB knowledge base for fuzzy / conceptual questions.

# Database schema (chat namespace)
Five tables; all use string UUID primary keys unless noted.

- chat.conversations(id, title, created_at, updated_at)
- chat.messages(id, conversation_id, role, body, status, created_at, updated_at)
    role:   'user' | 'assistant' | 'system'
    status: 'pending' | 'streaming' | 'final' | 'cancelled' | 'error'
- chat.typing_tokens(id, conversation_id, message_id, body, seq, created_at)
    Streaming token deltas. TTL-expired by KalamDB.
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
   IMMEDIATELY BEFORE the destructive tool, in the same turn.

   NEVER ask for confirmation by replying with plain text like "Are you
   sure?" or "Please confirm with yes/no". The UI renders a real
   Approve/Reject dialog ONLY when you call the request_approval tool —
   asking in text leaves the user with a confusing chat message and no
   button. If the user uses ANY of the words delete / remove / wipe /
   purge / drop / erase / clear about their data, your VERY NEXT action
   is to call request_approval. This rule overrides any "ask for
   confirmation" instinct.

   Each request_approval covers EXACTLY ONE destructive tool call. The
   approval token is one-shot — it's consumed by the destructive tool
   that follows.

   Pick the right tool for the scope of the request:
   - Single-conversation delete ("delete this conversation",
     "delete conversation X") → request_approval (mentioning the
     specific conversation) → delete_conversation(id).
   - Bulk delete ("delete all my conversations", "wipe everything",
     "clear my history") → request_approval (mentioning the bulk
     scope, e.g. "Permanently delete ALL of your conversations?") →
     delete_all_conversations(). Do NOT loop delete_conversation for
     bulk requests — that forces the user to click Approve once per
     item.

   If approval returns 'rejected', stop and acknowledge in plain text.
   Never call delete_conversation without an approval that just returned
   'approved'.

4. Do not use request_approval for cosmetic confirmations or trivial
   clarifications (e.g. "should I respond in English or French?", "do
   you want bullet points?"). Those are plain-text questions.

5. Keep replies short unless the user asks for detail. Don't narrate your
   tool use ("Let me check the database..."); just produce the result.`;

interface Task {
  id: string;
  conversation_id: string;
  message_id: string;
  /** KalamDB user that owns this task. Sourced from the consumer change
   *  event's connecting-identity metadata. Every SQL the agent issues on
   *  behalf of this task is wrapped in `executeAsUser(sql, task.user, …)`
   *  so the per-user partitioning of the chat.* USER tables enforces
   *  tenant isolation end-to-end without the agent having to log in as
   *  every individual user. */
  user: string;
}

const activeControllers = new Map<string, AbortController>();

/** Map of in-flight `request_approval` waiters keyed by approval id. The
 *  approval-resolution consumer (see main()) calls the resolver when the
 *  matching `chat.approvals` row's status flips to approved / rejected. */
const pendingApprovals: PendingApprovals = new Map();

// Wire-format shapes for the three topics the agent consumes. KalamDB
// delivers consumer-event cells as plain JS primitives (via JSON), so
// these types narrow `change.data.X` from `unknown` to the actual
// runtime type — typo protection on field names + correct value types
// at the call site. No `unwrap()` needed for consumer events; that
// helper is for `client.query()` results, which use a different layer.
type TaskEventRow = {
  id: string;
  conversation_id: string;
  message_id: string;
};

type TaskCancelRow = {
  id: string;
  is_cancelled: boolean;
};

type ApprovalResolutionRow = {
  id: string;
  status: string;
};

async function main(): Promise<void> {
  // One consumer client for all three topics (task events + cancels +
  // approval resolutions). Topic membership is global, not per-tenant.
  const consumerClient = createConsumerClient({
    url: KALAMDB_URL,
    authProvider: async () => Auth.basic(ADMIN_USER, ADMIN_PASSWORD),
    disableCompression: true,
  });
  // One SQL client, authenticated as the admin. Every DML / query the agent
  // issues on behalf of a task is wrapped in
  // `executeAsUser(sql, task.user, …)`, which KalamDB rewrites to
  // `EXECUTE AS USER '<user>' (<sql>)` server-side. Single connection
  // replaces the previous per-user client pool.
  const sqlClient = createClient({
    url: KALAMDB_URL,
    authProvider: async () => Auth.basic(ADMIN_USER, ADMIN_PASSWORD),
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
  log.info(
    {
      url: KALAMDB_URL,
      llm: llm.name,
      topics: { task: TASK_TOPIC, cancel: CANCEL_TOPIC, approval: APPROVAL_TOPIC },
      group: GROUP_ID,
    },
    "agent ready",
  );

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

  // Three consumers run concurrently:
  //
  //   1. chat.task_events (ON INSERT)   — work queue. One agent process in
  //      the group picks up each new task and drives the LLM.
  //   2. chat.task_cancels (ON UPDATE)  — Stop-button signal. One consumer
  //      receives every cancellation across all users and aborts the
  //      matching local AbortController.
  //   3. chat.approval_resolutions      — Approve/Reject signal. Resolves
  //      pending `request_approval` waiters when the UI flips the approval
  //      row's status.
  //
  // Each pair (cancels, approvals) is GLOBAL across users — that's the
  // architectural win over per-task live() subscriptions: O(1) connections
  // instead of O(open-tasks).
  await Promise.all([
    runTaskConsumer(consumerClient, sqlClient, llm, stop.signal),
    runCancelConsumer(consumerClient, stop.signal),
    runApprovalResolutionConsumer(consumerClient, stop.signal),
  ]);
}

async function runTaskConsumer(
  consumerClient: ReturnType<typeof createConsumerClient>,
  sqlClient: KalamDBClient,
  llm: LlmAdapter,
  stopSignal: AbortSignal,
): Promise<void> {
  await runConsumer<TaskEventRow>({
    client: consumerClient,
    name: "chat-agent",
    topic: TASK_TOPIC,
    groupId: GROUP_ID,
    start: "earliest",
    batchSize: 10,
    timeoutSeconds: 30,
    stopSignal,
    onChange: async (
      _ctx: ConsumerRunContext<TaskEventRow>,
      change: ConsumerChange<TaskEventRow>,
    ) => {
      const task: Task = {
        id: change.data.id,
        conversation_id: change.data.conversation_id,
        message_id: change.data.message_id,
        user: String(change.user ?? ""),
      };
      if (
        !UUID_RE.test(task.id) ||
        !UUID_RE.test(task.conversation_id) ||
        !UUID_RE.test(task.message_id) ||
        !USER_RE.test(task.user)
      ) {
        log.warn({ row: change.data, user: task.user }, "dropping malformed task event");
        return;
      }
      if (activeControllers.has(task.id)) return;

      // At-least-once redelivery: skip tasks already finished, cancelled,
      // or completely deleted. If the row exists but is cancelled-not-
      // finalized (Stop clicked before any agent attached, or a prior
      // attempt crashed), finalize it so the UI clears.
      // If the row was deleted entirely (test cleanup, manual purge),
      // isTaskTerminal returns null — treat that as "nothing to do" and
      // skip, otherwise the agent rebuilds the assistant stub against a
      // conversation that no longer exists.
      const terminal = await isTaskTerminal(sqlClient, task.user, task.id, log);
      if (terminal === null) return;
      if (terminal === true) {
        await finalizeStuckCancelled(sqlClient, task.user, task.id, task.message_id, log).catch(
          () => undefined,
        );
        return;
      }

      const ctrl = new AbortController();
      activeControllers.set(task.id, ctrl);
      try {
        await runTask(sqlClient, llm, task, ctrl);
      } finally {
        activeControllers.delete(task.id);
      }
    },
    onConnectionRetry: ({ attempt, backoffMs, error }) => {
      log.warn({ attempt, backoffMs, err: error }, "task consumer reconnecting");
    },
    onConnectionRestored: ({ attempt }) => {
      log.info({ attempt }, "task consumer reconnected");
    },
    onConnectionError: ({ error, attempt }) => {
      log.error({ attempt, err: error }, "task consumer giving up");
    },
  });
}

async function runCancelConsumer(
  consumerClient: ReturnType<typeof createConsumerClient>,
  stopSignal: AbortSignal,
): Promise<void> {
  // Replaces the previous N per-task live() subscriptions. One consumer
  // here receives every chat.tasks UPDATE WHERE is_cancelled=true across
  // every user; we look the task_id up in activeControllers and abort.
  await runConsumer<TaskCancelRow>({
    client: consumerClient,
    name: "chat-agent-cancels",
    topic: CANCEL_TOPIC,
    // Each replica needs its OWN view of cancels (the abort signal is
    // local to the process holding the task), so we use a unique group
    // per process. With a shared group only ONE replica would see the
    // cancel and the others' tasks would run on.
    groupId: `chat-agents-cancels-${process.pid}`,
    start: "latest",
    // batchSize=1 so a user-clicked Stop fires the consumer's onChange
    // the instant the event arrives, instead of waiting for a batch to
    // accumulate. Cancels are infrequent + latency-sensitive — the user
    // is staring at the UI waiting for the bubble to mark "(stopped)".
    // timeoutSeconds is the long-poll re-issue cadence when idle (every
    // 30s the consumer re-polls; default is fine since batchSize=1 still
    // returns immediately on event arrival).
    batchSize: 1,
    timeoutSeconds: 30,
    stopSignal,
    onChange: async (
      _ctx: ConsumerRunContext<TaskCancelRow>,
      change: ConsumerChange<TaskCancelRow>,
    ) => {
      const taskId = change.data.id;
      if (!UUID_RE.test(taskId)) return;
      // Defense-in-depth + back-compat with KalamDB ≤ 0.5.0-beta.1.
      // The topic source `ON UPDATE WHERE is_cancelled = true` filter
      // was added in a later main commit (kalamdb-row-filter integration
      // in the publisher routing). On the older pinned image, every
      // UPDATE to chat.tasks fires this topic — including the agent's
      // own finalizeTask. Re-checking is_cancelled here keeps the
      // cascade correct on both old and new server versions.
      if (change.data.is_cancelled !== true) return;
      const ctrl = activeControllers.get(taskId);
      if (!ctrl) return; // task isn't running on this replica
      if (ctrl.signal.aborted) return;
      log.info({ task_id: taskId, user: String(change.user ?? "") }, "cancel signal received");
      ctrl.abort();
    },
    onConnectionError: ({ error, attempt }) => {
      log.error({ attempt, err: error }, "cancel consumer giving up");
    },
  });
}

async function runApprovalResolutionConsumer(
  consumerClient: ReturnType<typeof createConsumerClient>,
  stopSignal: AbortSignal,
): Promise<void> {
  // Replaces the previous per-approval live() subscription inside
  // request_approval. The tool registers a resolver in pendingApprovals
  // keyed by approval id; we fire it when the matching topic event lands.
  await runConsumer<ApprovalResolutionRow>({
    client: consumerClient,
    name: "chat-agent-approvals",
    topic: APPROVAL_TOPIC,
    groupId: `chat-agents-approvals-${process.pid}`,
    start: "latest",
    // Same shape as the cancel consumer — Approve/Reject clicks are
    // latency-sensitive (user is staring at the UI waiting for the
    // next agent step). See cancel-consumer comment above for the
    // batchSize=1 + timeoutSeconds=30 rationale.
    batchSize: 1,
    timeoutSeconds: 30,
    stopSignal,
    onChange: async (
      _ctx: ConsumerRunContext<ApprovalResolutionRow>,
      change: ConsumerChange<ApprovalResolutionRow>,
    ) => {
      const approvalId = change.data.id;
      const status = change.data.status;
      if (!UUID_RE.test(approvalId)) return;
      if (status !== "approved" && status !== "rejected") return;
      const resolver = pendingApprovals.get(approvalId);
      if (!resolver) return; // not waiting on this approval on this replica
      log.info({ approval_id: approvalId, status }, "approval resolved");
      resolver(status);
    },
    onConnectionError: ({ error, attempt }) => {
      log.error({ attempt, err: error }, "approval consumer giving up");
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
    user: task.user,
  });
  tlog.info("task started");

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
    // No clearTypingTokens — chat.typing_tokens is a STREAM table with TTL.
    await finalizeTask(client, task.user, task.id).catch(() => undefined);
    tlog.info({ final_status: finalStatus }, "task complete");
  };

  try {
    await ensureAssistantStub(client, task);
    await markStreaming(client, task.user, task.message_id);

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
    baseMessages.push(...(await fetchHistory(client, task.user, task.conversation_id)));
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
        await client.executeAsUser(
          `INSERT INTO chat.typing_tokens (id, conversation_id, message_id, body, seq, created_at)
           VALUES ($1, $2, $3, $4, $5, $6)`,
          task.user,
          [
            crypto.randomUUID(),
            task.conversation_id,
            task.message_id,
            body,
            ++seq,
            new Date().toISOString(),
          ],
        );
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
      pendingApprovals,
    };
    let turn = 0;
    while (turn < MAX_TOOL_TURNS) {
      // A cancel could land between the previous turn's last abort check and
      // this iteration (e.g. between markStreaming and llm.stream, or between
      // a tool call returning and the next LLM round). Without this, the
      // agent would fire one more LLM request before noticing.
      if (controller.signal.aborted) {
        finalStatus = "cancelled";
        await finalizeMessage(client, task.user, task.message_id, assembled.join(""), "cancelled");
        return;
      }
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
          await finalizeMessage(
            client,
            task.user,
            task.message_id,
            assembled.join(""),
            "cancelled",
          );
          return;
        }
        tlog.error({ err: error }, "llm error");
        finalStatus = "error";
        await finalizeMessage(
          client,
          task.user,
          task.message_id,
          "Sorry — something went wrong.",
          "error",
        );
        return;
      }

      if (controller.signal.aborted) {
        finalStatus = "cancelled";
        await finalizeMessage(client, task.user, task.message_id, assembled.join(""), "cancelled");
        return;
      }

      if (stopReason !== "tool_calls" || pendingCalls.length === 0) {
        await flush();
        finalStatus = "final";
        await finalizeMessage(client, task.user, task.message_id, assembled.join(""), "final");
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
          await finalizeMessage(
            client,
            task.user,
            task.message_id,
            assembled.join(""),
            "cancelled",
          );
          return;
        }
      }
    }

    finalStatus = "final";
    await finalizeMessage(
      client,
      task.user,
      task.message_id,
      assembled.join("") + "\n\n(Agent exceeded tool-call turn limit.)",
      "final",
    );
  } finally {
    await cleanup();
  }
}

async function fetchHistory(
  client: KalamDBClient,
  user: string,
  conversationId: string,
): Promise<LlmMessage[]> {
  const res = await client.executeAsUser(
    `SELECT role, body FROM chat.messages
       WHERE conversation_id = $1
         AND status IN ('final', 'streaming')
       ORDER BY created_at ASC`,
    user,
    [conversationId],
  );
  const out: LlmMessage[] = [];
  for (const row of extractRows(res)) {
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
  user: string,
  taskId: string,
  log_: typeof log,
): Promise<boolean | null> {
  const res = await client.executeAsUser(
    `SELECT finished_at, is_cancelled FROM chat.tasks WHERE id = $1`,
    user,
    [taskId],
  );
  const row = extractRows(res)[0];
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
  const res = await client.executeAsUser(`SELECT id FROM chat.messages WHERE id = $1`, task.user, [
    task.message_id,
  ]);
  if (extractRows(res).length > 0) return;
  const now = new Date().toISOString();
  try {
    await client.executeAsUser(
      `INSERT INTO chat.messages (id, conversation_id, role, body, status, created_at, updated_at)
       VALUES ($1, $2, $3, $4, $5, $6, $7)`,
      task.user,
      [task.message_id, task.conversation_id, "assistant", "", "pending", now, now],
    );
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    if (/duplicate|already exists|unique|primary key/i.test(msg)) {
      log.warn({ message_id: task.message_id }, "assistant stub already exists (race) — ignored");
      return;
    }
    throw err;
  }
}

async function markStreaming(
  client: KalamDBClient,
  user: string,
  messageId: string,
): Promise<void> {
  await client.executeAsUser(
    `UPDATE chat.messages SET status = $1, updated_at = $2 WHERE id = $3`,
    user,
    ["streaming", new Date().toISOString(), messageId],
  );
}

async function finalizeMessage(
  client: KalamDBClient,
  user: string,
  messageId: string,
  body: string,
  status: "final" | "cancelled" | "error",
): Promise<void> {
  await client.executeAsUser(
    `UPDATE chat.messages SET body = $1, status = $2, updated_at = $3 WHERE id = $4`,
    user,
    [body, status, new Date().toISOString(), messageId],
  );
}

async function finalizeTask(client: KalamDBClient, user: string, taskId: string): Promise<void> {
  await client.executeAsUser(`UPDATE chat.tasks SET finished_at = $1 WHERE id = $2`, user, [
    new Date().toISOString(),
    taskId,
  ]);
}

/**
 * Finalizes a task that arrived already-cancelled (user clicked Stop before
 * the agent could attach). Without this, the task row's finished_at stays
 * null forever and the UI keeps the composer disabled — `isAgentBusy` in
 * App.tsx treats `!finished_at` as "still working".
 *
 * Also marks the assistant message as `cancelled` if it exists in a
 * non-terminal state, so the bubble shows the "(stopped)" marker instead of
 * the typing indicator.
 */
async function finalizeStuckCancelled(
  client: KalamDBClient,
  user: string,
  taskId: string,
  messageId: string,
  log_: typeof log,
): Promise<void> {
  log_.info({ task_id: taskId }, "finalizing stuck cancelled task");
  await client.executeAsUser(`UPDATE chat.tasks SET finished_at = $1 WHERE id = $2`, user, [
    new Date().toISOString(),
    taskId,
  ]);
  // The assistant message stub may or may not exist (depends on whether a
  // prior delivery attempt got far enough to insert it). UPDATE on a
  // missing row is a no-op, and the status filter keeps us from clobbering
  // a message that's already final.
  await client
    .executeAsUser(
      `UPDATE chat.messages
         SET status = 'cancelled', updated_at = $1
       WHERE id = $2 AND status IN ('pending', 'streaming')`,
      user,
      [new Date().toISOString(), messageId],
    )
    .catch(() => undefined);
}

main().catch((err) => {
  log.fatal({ err }, "agent fatal");
  process.exit(1);
});
