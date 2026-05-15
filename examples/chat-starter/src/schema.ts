import { boolean, integer, text, timestamp } from "drizzle-orm/pg-core";
import { kTable } from "@kalamdb/orm";

export const conversations = kTable.user("chat.conversations", {
  id: text("id").primaryKey(),
  title: text("title").notNull(),
  createdAt: timestamp("created_at").notNull(),
  updatedAt: timestamp("updated_at").notNull(),
});

export const messages = kTable.user("chat.messages", {
  id: text("id").primaryKey(),
  conversationId: text("conversation_id").notNull(),
  role: text("role").notNull(),
  body: text("body").notNull(),
  status: text("status").notNull(),
  createdAt: timestamp("created_at").notNull(),
  updatedAt: timestamp("updated_at").notNull(),
});

export const typingTokens = kTable.user("chat.typing_tokens", {
  id: text("id").primaryKey(),
  conversationId: text("conversation_id").notNull(),
  messageId: text("message_id").notNull(),
  body: text("body").notNull(),
  seq: integer("seq").notNull(),
  createdAt: timestamp("created_at").notNull(),
});

export const approvals = kTable.user("chat.approvals", {
  id: text("id").primaryKey(),
  conversationId: text("conversation_id").notNull(),
  messageId: text("message_id").notNull(),
  question: text("question").notNull(),
  status: text("status").notNull(),
  createdAt: timestamp("created_at").notNull(),
  resolvedAt: timestamp("resolved_at"),
});

export const tasks = kTable.user("chat.tasks", {
  id: text("id").primaryKey(),
  conversationId: text("conversation_id").notNull(),
  messageId: text("message_id").notNull(),
  isCancelled: boolean("is_cancelled").notNull(),
  startedAt: timestamp("started_at").notNull(),
  finishedAt: timestamp("finished_at"),
});
