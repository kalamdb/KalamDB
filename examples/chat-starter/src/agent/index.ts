import "dotenv/config";
import { Auth, createClient, type KalamDBClient, type Unsubscribe } from "@kalamdb/client";
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
  type LlmTool,
  type LlmToolCall,
} from "../lib/llm/index.js";
import { withRetry } from "../lib/llm/retry.js";
import { UUID_RE, uuidLit } from "./ids.js";
import { logger } from "../lib/logger.js";

const log = logger.child({ component: "agent" });

const KALAMDB_URL = process.env.KALAMDB_URL ?? "http://127.0.0.1:8080";
const USER = process.env.KALAMDB_USER ?? "root";
const PASSWORD = process.env.KALAMDB_PASSWORD ?? "kalamdb-dev-password";
const TASK_TOPIC = process.env.KALAMDB_TASK_TOPIC ?? "chat.task_events";
const GROUP_ID = process.env.KALAMDB_GROUP ?? "chat-agents";

const SYSTEM_PROMPT = `You are a concise, helpful assistant inside a KalamDB-powered chat app.

You have access to a single tool: \`request_approval(question)\`. Use it BEFORE taking any irreversible or risky action — for example: deleting data, sending emails, charging payments, running shell commands, sharing user data with third parties, or anything the user has not explicitly authorized in this turn.

The tool returns one of:
  - "approved": you may proceed; describe the action you took.
  - "rejected": stop immediately and acknowledge the user's decision.

Do not use the tool for cosmetic confirmations or trivial clarifications — ask the user in plain text for those.

Otherwise, keep replies short unless the user asks for more detail.`;

const APPROVAL_TOOL: LlmTool = {
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
  const llm = withRetry(await getLlmAdapter(), undefined, log);
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
      if (await isTaskTerminal(sqlClient, task.id)) {
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
  };

  try {
    await ensureAssistantStub(client, task);
    await markStreaming(client, task.message_id);

    const baseMessages: LlmMessage[] = [{ role: "system", content: SYSTEM_PROMPT }];
    baseMessages.push(...(await fetchHistory(client, task.conversation_id)));
    tlog.debug({ history: baseMessages.length - 1 }, "starting LLM turn");

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
    let turn = 0;
    while (turn < 8) {
      turn++;
      const pendingCalls: LlmToolCall[] = [];
      let stopReason: "stop" | "tool_calls" | "length" | "error" = "stop";

      try {
        for await (const event of llm.stream({
          messages,
          tools: [APPROVAL_TOOL],
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
        const result = await dispatchTool(client, task, call, controller.signal);
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
    void finalStatus;
    await cleanup();
  }
}

async function dispatchTool(
  client: KalamDBClient,
  task: Task,
  call: LlmToolCall,
  signal: AbortSignal,
): Promise<string> {
  if (call.name === "request_approval") {
    return await requestApproval(client, task, call, signal);
  }
  return `Unknown tool: ${call.name}`;
}

async function requestApproval(
  client: KalamDBClient,
  task: Task,
  call: LlmToolCall,
  signal: AbortSignal,
): Promise<string> {
  const question =
    typeof call.arguments.question === "string"
      ? call.arguments.question
      : "The assistant is requesting approval.";
  const approvalId = crypto.randomUUID();

  await client.insert("chat.approvals", {
    id: approvalId,
    conversation_id: task.conversation_id,
    message_id: task.message_id,
    question,
    status: "pending",
    created_at: new Date().toISOString(),
    resolved_at: null,
  });
  log.info({ approval_id: approvalId, message_id: task.message_id, question }, "approval pending");

  return await new Promise<string>((resolve) => {
    let settled = false;
    let unsubscribe: Unsubscribe | undefined;

    const finish = (value: string): void => {
      if (settled) return;
      settled = true;
      signal.removeEventListener("abort", onAbort);
      void unsubscribe?.();
      resolve(value);
    };
    const onAbort = (): void => finish("rejected (cancelled by user before approval resolved)");
    signal.addEventListener("abort", onAbort, { once: true });

    client
      .live<{ id: string; status: string }>(
        `SELECT id, status FROM chat.approvals WHERE id = ${uuidLit(approvalId)}`,
        (rows) => {
          const row = rows[0];
          if (!row) return;
          const status = unwrap(row.status);
          if (status === "approved" || status === "rejected") {
            log.info({ approval_id: approvalId, status }, "approval resolved");
            finish(status);
          }
        },
        {
          onError: (err) => {
            log.error({ approval_id: approvalId, err }, "approval live errored");
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

async function isTaskTerminal(client: KalamDBClient, taskId: string): Promise<boolean> {
  const res = await client.query(`SELECT finished_at, is_cancelled FROM chat.tasks WHERE id = $1`, [
    taskId,
  ]);
  const row = (res as { results?: Array<{ named_rows?: Array<Record<string, unknown>> }> })
    .results?.[0]?.named_rows?.[0];
  if (!row) return true;
  const finishedAt = unwrap(row.finished_at);
  const cancelled = unwrap(row.is_cancelled);
  return Boolean(finishedAt) || cancelled === true || cancelled === "true";
}

async function ensureAssistantStub(client: KalamDBClient, task: Task): Promise<void> {
  // Idempotent: if redelivery brought us back here, the row may already exist.
  const res = await client.query(`SELECT id FROM chat.messages WHERE id = $1`, [task.message_id]);
  const exists =
    ((res as { results?: Array<{ named_rows?: Array<unknown> }> }).results?.[0]?.named_rows
      ?.length ?? 0) > 0;
  if (exists) return;
  const now = new Date().toISOString();
  await client.insert("chat.messages", {
    id: task.message_id,
    conversation_id: task.conversation_id,
    role: "assistant",
    body: "",
    status: "pending",
    created_at: now,
    updated_at: now,
  });
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
