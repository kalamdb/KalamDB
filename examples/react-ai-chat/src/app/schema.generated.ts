import type { InferSelectModel } from 'drizzle-orm';
import { sql } from 'drizzle-orm';
import { text, timestamp } from 'drizzle-orm/pg-core';
import { file, kTable } from '@kalamdb/orm';

export const conversations = kTable.user('react_ai_chat.conversations', {
  id: text('id').primaryKey(),
  title: text('title').notNull(),
  summary: text('summary').notNull(),
  createdAt: timestamp('created_at', { mode: 'date' }).default(sql``).notNull(),
  updatedAt: timestamp('updated_at', { mode: 'date' }).default(sql``).notNull(),
});

export const messages = kTable.user('react_ai_chat.messages', {
  id: text('id').default(sql``).primaryKey(),
  clientId: text('client_id'),
  conversationId: text('conversation_id').notNull(),
  replyToMessageId: text('reply_to_message_id'),
  role: text('role').notNull(),
  body: text('body').notNull(),
  status: text('status').default(sql``).notNull(),
  attachment: file('attachment'),
  approvalId: text('approval_id'),
  createdAt: timestamp('created_at', { mode: 'date' }).default(sql``).notNull(),
  updatedAt: timestamp('updated_at', { mode: 'date' }).default(sql``).notNull(),
});

export const approvals = kTable.user('react_ai_chat.approvals', {
  id: text('id').primaryKey(),
  conversationId: text('conversation_id').notNull(),
  messageId: text('message_id').notNull(),
  title: text('title').notNull(),
  body: text('body').notNull(),
  status: text('status').default(sql``).notNull(),
  createdAt: timestamp('created_at', { mode: 'date' }).default(sql``).notNull(),
  updatedAt: timestamp('updated_at', { mode: 'date' }).default(sql``).notNull(),
});

export const typingTokens = kTable.stream('react_ai_chat.typing_tokens', {
  id: text('id').default(sql``).primaryKey(),
  conversationId: text('conversation_id').notNull(),
  messageId: text('message_id').notNull(),
  status: text('status').notNull(),
  token: text('token').default(sql``).notNull(),
  createdAt: timestamp('created_at', { mode: 'date' }).default(sql``).notNull(),
});

export const approvalActions = kTable.user('react_ai_chat.approval_actions', {
  id: text('id').default(sql``).primaryKey(),
  approvalId: text('approval_id').notNull(),
  conversationId: text('conversation_id').notNull(),
  action: text('action').notNull(),
  createdAt: timestamp('created_at', { mode: 'date' }).default(sql``).notNull(),
});

export type ConversationRow = InferSelectModel<typeof conversations>;
export type MessageRow = InferSelectModel<typeof messages>;
export type ApprovalRow = InferSelectModel<typeof approvals>;
export type TypingTokenRow = InferSelectModel<typeof typingTokens>;