import { and, desc, eq, sql } from "drizzle-orm";
import {
  defineProcedure,
  type ProcedureContext,
  type ChatDemoAiInbox,
  type ChatDemoSendMessageRequest,
  type ChatDemoSendMessageResult,
  chatDemoDirectMessages,
  chatDemoMessages,
} from "../generated/contracts";

function taggedRow<T extends ChatDemoAiInbox>(
  table: T["_table"],
  row: Omit<T, "_table"> | undefined,
): T {
  if (row == null) {
    throw new Error("send_message did not return a row");
  }
  return { _table: table, ...row } as T;
}

export default defineProcedure(
  async (ctx: ProcedureContext, input: ChatDemoSendMessageRequest): Promise<ChatDemoSendMessageResult> => {
    const targetId = input.target_id.trim();
    const content = input.content.trim();
    if (!targetId) {
      throw new Error("target_id is required");
    }
    if (!content) {
      throw new Error("content is required");
    }

    const actor = ctx.actor.id;
    if (input.target === "direct") {
      if (targetId !== actor) {
        throw new Error("direct target_id must be the current user");
      }
      await ctx.orm.insert(chatDemoDirectMessages).values({
        id: sql`DEFAULT`,
        role: "user",
        author: actor,
        sender_username: actor,
        content,
        created_at: sql`DEFAULT`,
      });
      const inserted = await ctx.orm
        .select({
          id: chatDemoDirectMessages.id,
          role: chatDemoDirectMessages.role,
          author: chatDemoDirectMessages.author,
          sender_username: chatDemoDirectMessages.sender_username,
          content: chatDemoDirectMessages.content,
          reply_to: chatDemoDirectMessages.reply_to,
        })
        .from(chatDemoDirectMessages)
        .where(
          and(eq(chatDemoDirectMessages.sender_username, actor), eq(chatDemoDirectMessages.content, content)),
        )
        .orderBy(desc(chatDemoDirectMessages.id))
        .limit(1);
      return taggedRow("chat_demo:direct_messages", inserted[0]);
    }

    await ctx.orm.insert(chatDemoMessages).values({
      id: sql`DEFAULT`,
      room: targetId,
      role: "user",
      author: actor,
      sender_username: actor,
      content,
      created_at: sql`DEFAULT`,
    });
    const inserted = await ctx.orm
      .select({
        id: chatDemoMessages.id,
        room: chatDemoMessages.room,
        role: chatDemoMessages.role,
        author: chatDemoMessages.author,
        sender_username: chatDemoMessages.sender_username,
        content: chatDemoMessages.content,
        reply_to: chatDemoMessages.reply_to,
      })
      .from(chatDemoMessages)
      .where(
        and(
          eq(chatDemoMessages.room, targetId),
          eq(chatDemoMessages.sender_username, actor),
          eq(chatDemoMessages.content, content),
        ),
      )
      .orderBy(desc(chatDemoMessages.id))
      .limit(1);
    return taggedRow("chat_demo:messages", inserted[0]);
  },
);
