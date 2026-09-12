import { and, desc, eq, sql } from "drizzle-orm";
import {
  procedure,
  type ChatDemoAiInbox,
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

export const sendMessage = procedure.chatDemo.sendMessage(async (ctx, input) => {
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
    const [row] = await ctx.orm
      .select()
      .from(chatDemoDirectMessages)
      .where(
        and(eq(chatDemoDirectMessages.sender_username, actor), eq(chatDemoDirectMessages.content, content)),
      )
      .orderBy(desc(chatDemoDirectMessages.id))
      .limit(1);
    return taggedRow("chat_demo:direct_messages", row);
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
  const [row] = await ctx.orm
    .select()
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
  return taggedRow("chat_demo:messages", row);
});
