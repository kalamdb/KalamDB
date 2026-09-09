import { and, eq, sql } from "drizzle-orm";
import {
  defineProcedure,
  type ProcedureContext,
  type ChatDemoAiInbox,
  type ChatDemoOnUserMessageRequest,
  type ChatDemoOnUserMessageResult,
  chatDemoAgentEvents,
  chatDemoDirectMessages,
  chatDemoMessages,
} from "../generated/contracts";

const THINKING_DELAY_MS = 200;
const STREAM_DELAY_MS = 80;
const STREAM_CHUNK_SIZE = 64;
const USER_NAME = /^[A-Za-z0-9._-]+$/;

function assertValidUser(user: string): string {
  if (!USER_NAME.test(user)) {
    throw new Error(`unsupported user for EXECUTE AS: ${user}`);
  }
  return user;
}

function isDirectPayload(
  payload: ChatDemoAiInbox,
): payload is Extract<ChatDemoAiInbox, { _table: "chat_demo:direct_messages" }> {
  return payload._table === "chat_demo:direct_messages";
}

function asBigInt(value: unknown): bigint {
  if (typeof value === "bigint") {
    return value;
  }
  if (typeof value === "number" && Number.isFinite(value)) {
    return BigInt(Math.trunc(value));
  }
  return BigInt(String(value));
}

function inboxPayload(input: ChatDemoOnUserMessageRequest): ChatDemoAiInbox {
  const raw: unknown = input.payload;
  if (typeof raw === "string") {
    return JSON.parse(raw) as ChatDemoAiInbox;
  }
  if (raw && typeof raw === "object") {
    return raw as ChatDemoAiInbox;
  }
  throw new Error("on_user_message expected input.payload");
}

export function buildReply(content: string): string {
  const trimmed = content.trim();
  const lowered = trimmed.toLowerCase();
  let extra =
    "This demo uses a schema-first topic trigger: chat_demo.on_user_message drafts the reply inside KalamDB — no separate agent process.";

  if (lowered.includes("latency")) {
    extra =
      "I would inspect the slowest route first, then compare the latest write volume against the baseline you see in the database.";
  } else if (lowered.includes("deploy")) {
    extra = "That sounds deploy-shaped. I would compare the newest release marker against the first spike in user messages.";
  } else if (lowered.includes("queue")) {
    extra =
      "Queue growth usually means a downstream dependency is flattening. This is a good fit for another procedure wired to a different topic.";
  }

  return `AI reply: KalamDB stored "${trimmed}" in the chat transcript, streamed the drafting state through chat_demo.agent_events, and committed the final reply as the procedure principal. ${extra}`;
}

export default defineProcedure(
  async (ctx: ProcedureContext, input: ChatDemoOnUserMessageRequest): Promise<ChatDemoOnUserMessageResult> => {
    const payload = inboxPayload(input);
    if (payload.role !== "user") {
      return;
    }

    const sender = assertValidUser(payload.sender_username.trim());
    const content = payload.content;
    const messageId = payload.id;

    const direct = isDirectPayload(payload);
    const scope = direct ? "direct" : "room";
    const destination = direct ? sender : payload.room;
    const responseId = `reply-${messageId}`;
    const asSender = ctx.orm.as(sender);
    const already = direct
      ? await asSender
          .select({ id: chatDemoDirectMessages.id })
          .from(chatDemoDirectMessages)
          .where(
            and(eq(chatDemoDirectMessages.reply_to, messageId), eq(chatDemoDirectMessages.role, "assistant")),
          )
          .limit(1)
      : await ctx.orm
          .select({ id: chatDemoMessages.id })
          .from(chatDemoMessages)
          .where(and(eq(chatDemoMessages.reply_to, messageId), eq(chatDemoMessages.role, "assistant")))
          .limit(1);
    if (already.length > 0) {
      return;
    }

    const emit = async (stage: string, preview: string, message: string): Promise<void> => {
      await asSender.insert(chatDemoAgentEvents).values({
        id: sql`DEFAULT`,
        response_id: responseId,
        room: destination,
        scope,
        sender_username: sender,
        stage,
        preview,
        message,
        created_at: sql`DEFAULT`,
      });
    };

    ctx.log.info("picked up user message", { id: messageId, sender, scope, destination });
    await emit("log", "", `Picked up user message ${messageId}`);
    await emit("thinking", "", "Planning assistant reply");
    await ctx.sleep(THINKING_DELAY_MS);

    const reply = buildReply(content);
    let streamedReply = "";
    for (let index = 0; index < reply.length; index += STREAM_CHUNK_SIZE) {
      streamedReply += reply.slice(index, index + STREAM_CHUNK_SIZE);
      await emit("typing", streamedReply, `Streamed ${streamedReply.length}/${reply.length} characters`);
      await ctx.sleep(STREAM_DELAY_MS);
    }

    await emit("log", streamedReply, "Persisting final assistant reply");
    if (direct) {
      await asSender.insert(chatDemoDirectMessages).values({
        id: sql`DEFAULT`,
        role: "assistant",
        author: "KalamDB Copilot",
        sender_username: sender,
        content: reply,
        reply_to: messageId,
        created_at: sql`DEFAULT`,
      });
    } else {
      await ctx.orm.insert(chatDemoMessages).values({
        id: sql`DEFAULT`,
        room: destination,
        role: "assistant",
        author: "KalamDB Copilot",
        sender_username: sender,
        content: reply,
        reply_to: messageId,
        created_at: sql`DEFAULT`,
      });
    }
    await emit("message_saved", reply, "Assistant reply committed");
    await ctx.sleep(STREAM_DELAY_MS);
    await emit("complete", reply, "Live stream finished");
  },
);
