import "dotenv/config";
import {
  Auth,
  createClient,
  type KalamDBClient,
  type Unsubscribe,
} from "@kalamdb/client";
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

const URL = process.env.KALAMDB_URL ?? "http://127.0.0.1:8080";
const USER = process.env.KALAMDB_USER ?? "root";
const PASSWORD = process.env.KALAMDB_PASSWORD ?? "kalamdb-dev-password";
const TASK_TOPIC = "chat.task_events";
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
  is_cancelled: boolean;
  finished_at: string | null;
}

const activeControllers = new Map<string, AbortController>();

async function main(): Promise<void> {
  // Two clients: the consumer client drives runConsumer (topic pulls + acks),
  // and a regular KalamDB client handles SQL + live queries (cancellation
  // channel + approval-resolution subscriptions).
  const consumerClient = createConsumerClient({
    url: URL,
    authProvider: async () => Auth.basic(USER, PASSWORD),
    disableCompression: true,
  });
  const sqlClient = createClient({
    url: URL,
    authProvider: async () => Auth.basic(USER, PASSWORD),
    disableCompression: true,
  });
  await sqlClient.connect();
  const llm = await getLlmAdapter();
  console.log(`[agent] connected to ${URL}, using ${llm.name}`);
  console.log(`[agent] topic=${TASK_TOPIC} group=${GROUP_ID}`);

  process.on("unhandledRejection", (reason) => {
    console.error("[agent] unhandled rejection:", reason);
  });

  // Cancellation channel: live query on chat.tasks streams UPDATE events so
  // we can abort an in-flight task when its is_cancelled flips to true.
  // (runConsumer is INSERT-sourced, so it can't carry cancel signals.)
  const cancelUnsub = await sqlClient.live<Task>(
    "SELECT id, is_cancelled FROM chat.tasks",
    (rows) => {
      for (const row of rows) {
        const id = unwrap(row.id);
        const cancelled = unwrap(row.is_cancelled);
        if (cancelled !== true && cancelled !== "true") continue;
        const ctrl = activeControllers.get(id);
        if (ctrl && !ctrl.signal.aborted) {
          console.log(`[agent] cancel signal for ${id.slice(0, 8)}`);
          ctrl.abort();
        }
      }
    },
  );

  const stop = new AbortController();
  process.on("SIGINT", async () => {
    console.log("\n[agent] shutting down");
    stop.abort();
    for (const ctrl of activeControllers.values()) ctrl.abort();
    await cancelUnsub();
    await sqlClient.disconnect();
    process.exit(0);
  });

  // Work queue: runConsumer pulls new tasks off the topic with at-least-once
  // delivery + group-based load balancing across agent replicas.
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
        is_cancelled: false,
        finished_at: null,
      };
      if (!task.id || activeControllers.has(task.id)) return;
      const ctrl = new AbortController();
      activeControllers.set(task.id, ctrl);
      try {
        await runTask(sqlClient, llm, task, ctrl);
      } finally {
        activeControllers.delete(task.id);
      }
    },
    onConnectionRetry: ({ attempt, backoffMs, error }) => {
      console.warn(`[agent] reconnecting in ${backoffMs}ms (attempt ${attempt}): ${error instanceof Error ? error.message : String(error)}`);
    },
    onConnectionRestored: ({ attempt }) => {
      console.log(`[agent] reconnected after ${attempt} attempt(s)`);
    },
  });
}

async function runTask(
  client: KalamDBClient,
  llm: LlmAdapter,
  task: Task,
  controller: AbortController,
): Promise<void> {
  console.log(`[agent] task ${task.id.slice(0, 8)} → message ${task.message_id.slice(0, 8)}`);

  try {
    await markStreaming(client, task.message_id);
    const baseMessages: LlmMessage[] = [{ role: "system", content: SYSTEM_PROMPT }];
    baseMessages.push(...(await fetchHistory(client, task.conversation_id)));
    console.log(`[agent] history: ${baseMessages.length - 1} messages; starting LLM turn`);

    const assembled: string[] = [];
    let seq = 0;
    const messages: LlmMessage[] = [...baseMessages];
    let turn = 0;

    let buffer = "";
    let flushTimer: NodeJS.Timeout | null = null;
    const flush = async () => {
      if (flushTimer) { clearTimeout(flushTimer); flushTimer = null; }
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
        console.error("[agent] typing-token flush failed:", (e as Error).message);
      }
    };
    const enqueue = (delta: string) => {
      buffer += delta;
      if (buffer.length >= 16) {
        void flush();
      } else if (!flushTimer) {
        flushTimer = setTimeout(() => { void flush(); }, 200);
      }
    };

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
          await finalizeMessage(client, task.message_id, assembled.join(""), "cancelled");
          return;
        }
        console.error(`[agent] llm error:`, error);
        await finalizeMessage(client, task.message_id, "Sorry — something went wrong.", "error");
        return;
      }

      if (controller.signal.aborted) {
        await finalizeMessage(client, task.message_id, assembled.join(""), "cancelled");
        return;
      }

      if (stopReason !== "tool_calls" || pendingCalls.length === 0) {
        await flush();
        await finalizeMessage(client, task.message_id, assembled.join(""), "final");
        await clearTypingTokens(client, task.message_id);
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
          await finalizeMessage(client, task.message_id, assembled.join(""), "cancelled");
          return;
        }
      }
    }

    await finalizeMessage(
      client,
      task.message_id,
      assembled.join("") + "\n\n(Agent exceeded tool-call turn limit.)",
      "final",
    );
    await clearTypingTokens(client, task.message_id);
  } finally {
    await finalizeTask(client, task.id);
  }
}

async function dispatchTool(
  client: KalamDBClient,
  task: { id: string; conversation_id: string; message_id: string },
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
  task: { id: string; conversation_id: string; message_id: string },
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
  console.log(`[agent] approval ${approvalId.slice(0, 8)} pending: ${question}`);

  return await new Promise<string>((resolve) => {
    let settled = false;
    let unsubscribe: Unsubscribe | undefined;

    const onAbort = () => {
      if (settled) return;
      settled = true;
      void unsubscribe?.();
      resolve("rejected (cancelled by user before approval resolved)");
    };
    signal.addEventListener("abort", onAbort, { once: true });

    client
      .live<{ id: string; status: string }>(
        `SELECT id, status FROM chat.approvals WHERE id = '${approvalId}'`,
        (rows) => {
          const row = rows[0];
          if (!row) return;
          const status = unwrap(row.status);
          if (status === "approved" || status === "rejected") {
            if (settled) return;
            settled = true;
            signal.removeEventListener("abort", onAbort);
            void unsubscribe?.();
            console.log(`[agent] approval ${approvalId.slice(0, 8)} resolved: ${status}`);
            resolve(status);
          }
        },
      )
      .then((u) => {
        unsubscribe = u;
        if (settled) void u();
      })
      .catch((err) => {
        if (settled) return;
        settled = true;
        signal.removeEventListener("abort", onAbort);
        resolve(`error subscribing to approval: ${String(err)}`);
      });
  });
}

async function fetchHistory(
  client: KalamDBClient,
  conversationId: string,
): Promise<LlmMessage[]> {
  const res = await client.query(
    `SELECT role, body FROM chat.messages
       WHERE conversation_id = '${conversationId}'
         AND status IN ('final', 'streaming')
       ORDER BY created_at ASC`,
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
  const res = await client.query(
    `SELECT id FROM chat.typing_tokens WHERE message_id = '${messageId}'`,
  );
  const rows =
    (res as { results?: Array<{ named_rows?: Array<Record<string, unknown>> }> }).results?.[0]
      ?.named_rows ?? [];
  for (const row of rows) {
    const id = unwrap(row.id);
    if (typeof id === "string" && id.length > 0) {
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
  console.error("[agent] fatal:", err);
  process.exit(1);
});
