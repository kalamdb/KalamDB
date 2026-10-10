import { and, desc, eq, sql } from "drizzle-orm";
import {
  procedure,
  type AiInbox,
  directMessages,
  messages,
} from "../generated/contracts";

function taggedRow<T extends AiInbox>(
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
    await ctx.orm.insert(directMessages).values({
      id: sql`DEFAULT`,
      role: "user",
      author: actor,
      sender_username: actor,
      content,
      created_at: sql`DEFAULT`,
    });
    const [row] = await ctx.orm
      .select()
      .from(directMessages)
      .where(
        and(eq(directMessages.sender_username, actor), eq(directMessages.content, content)),
      )
      .orderBy(desc(directMessages.id))
      .limit(1);
    return taggedRow("chat_demo:direct_messages", row);
  }

  await ctx.orm.insert(messages).values({
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
    .from(messages)
    .where(
      and(
        eq(messages.room, targetId),
        eq(messages.sender_username, actor),
        eq(messages.content, content),
      ),
    )
    .orderBy(desc(messages.id))
    .limit(1);
  return taggedRow("chat_demo:messages", row);
});
