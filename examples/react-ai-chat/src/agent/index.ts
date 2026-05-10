import { config as loadEnv } from 'dotenv';
import { fileURLToPath } from 'node:url';
import { Auth, type KalamDBClient } from '@kalamdb/client';
import { createConsumerClient, runAgent } from '@kalamdb/consumer';
export { buildApprovalMessage, buildAssistantReply, createToolPlan, splitIntoTokenChunks } from './logic';
import { buildApprovalMessage, buildAssistantReply, createToolPlan, splitIntoTokenChunks } from './logic';

loadEnv({ path: '.env.local', quiet: true });
loadEnv({ quiet: true });

const KALAMDB_URL = process.env.KALAMDB_URL ?? 'http://127.0.0.1:8080';
const KALAMDB_USER = process.env.KALAMDB_USER ?? 'admin';
const KALAMDB_PASSWORD = process.env.KALAMDB_PASSWORD ?? 'kalamdb123';
const MESSAGE_TOPIC = 'react_ai_chat.agent_messages';
const ACTION_TOPIC = 'react_ai_chat.agent_actions';
const STREAM_DELAY_MS = 1_000;

type TopicRow = Record<string, unknown>;

function field(row: TopicRow, key: string): string {
  const value = row[key];
  return typeof value === 'string' ? value : value == null ? '' : String(value);
}

function validUser(user: string): string {
  if (!/^[A-Za-z0-9._-]+$/.test(user)) {
    throw new Error(`Unsupported KalamDB user: ${user}`);
  }
  return user;
}

async function sleep(ms: number): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, ms));
}

async function runSqlAsUser(client: KalamDBClient, sql: string, params?: unknown[]): Promise<void> {
  await client.executeAsUser(sql, validUser(KALAMDB_USER), params);
}

async function insertTypingToken(
  client: KalamDBClient,
  conversationId: string,
  messageId: string,
  status: string,
  token: string,
): Promise<void> {
  await runSqlAsUser(
    client,
    'INSERT INTO react_ai_chat.typing_tokens (conversation_id, message_id, status, token) VALUES ($1, $2, $3, $4)',
    [conversationId, messageId, status, token],
  );
}

async function insertAssistantMessage(
  client: KalamDBClient,
  values: {
    clientId?: string;
    conversationId: string;
    replyToMessageId: string;
    body: string;
    status: string;
    approvalId?: string;
  },
): Promise<void> {
  await runSqlAsUser(
    client,
    'INSERT INTO react_ai_chat.messages (client_id, conversation_id, reply_to_message_id, role, body, status, approval_id) VALUES ($1, $2, $3, $4, $5, $6, $7)',
    [
      values.clientId ?? null,
      values.conversationId,
      values.replyToMessageId,
      'assistant',
      values.body,
      values.status,
      values.approvalId ?? null,
    ],
  );
}

async function insertApproval(
  client: KalamDBClient,
  values: {
    id: string;
    conversationId: string;
    messageId: string;
    title: string;
    body: string;
  },
): Promise<void> {
  await runSqlAsUser(
    client,
    'INSERT INTO react_ai_chat.approvals (id, conversation_id, message_id, title, body, status) VALUES ($1, $2, $3, $4, $5, $6)',
    [values.id, values.conversationId, values.messageId, values.title, values.body, 'pending'],
  );
}

async function updateApproval(client: KalamDBClient, approvalId: string, status: string): Promise<void> {
  await runSqlAsUser(
    client,
    'UPDATE react_ai_chat.approvals SET status = $1, updated_at = NOW() WHERE id = $2',
    [status, approvalId],
  );
}

async function readApproval(client: KalamDBClient, approvalId: string): Promise<TopicRow | null> {
  const rows = await client.queryAll(
    `EXECUTE AS USER '${validUser(KALAMDB_USER)}' (SELECT * FROM react_ai_chat.approvals WHERE id = $1)`,
    [approvalId],
  );
  const row = rows[0];
  if (!row) {
    return null;
  }

  return Object.fromEntries(Object.entries(row).map(([key, value]) => [key, cellString(value)]));
}

async function streamThenInsertReply(
  client: KalamDBClient,
  conversationId: string,
  replyToMessageId: string,
  body: string,
): Promise<void> {
  const draftMessageId = `draft-${replyToMessageId}-${Date.now()}`;
  await insertTypingToken(client, conversationId, draftMessageId, 'thinking', 'Thinking through the next step. ');
  await sleep(STREAM_DELAY_MS);

  for (const token of splitIntoTokenChunks(body)) {
    await insertTypingToken(client, conversationId, draftMessageId, 'typing', token);
    await sleep(STREAM_DELAY_MS);
  }

  await insertTypingToken(client, conversationId, draftMessageId, 'saving', 'Saving final answer.');
  await insertAssistantMessage(client, {
    clientId: draftMessageId,
    conversationId,
    replyToMessageId,
    body,
    status: 'complete',
  });
}

async function handleUserMessage(client: KalamDBClient, row: TopicRow): Promise<void> {
  if (field(row, 'role') !== 'user' || field(row, 'status') !== 'sent') {
    return;
  }

  const messageId = field(row, 'id');
  const conversationId = field(row, 'conversation_id');
  const body = field(row, 'body');
  const plan = createToolPlan(body);

  if (plan.requiresApproval) {
    const approvalId = `approval-${messageId}`;
    await insertApproval(client, {
      id: approvalId,
      conversationId,
      messageId,
      title: plan.approvalTitle,
      body: plan.approvalBody,
    });
    await insertAssistantMessage(client, {
      conversationId,
      replyToMessageId: messageId,
      body: buildApprovalMessage(body),
      status: 'awaiting_approval',
      approvalId,
    });
    return;
  }

  await streamThenInsertReply(client, conversationId, messageId, buildAssistantReply(body));
}

async function handleApprovalAction(client: KalamDBClient, row: TopicRow): Promise<void> {
  const action = field(row, 'action');
  const approvalId = field(row, 'approval_id');
  const conversationId = field(row, 'conversation_id');
  const approval = await readApproval(client, approvalId);
  const sourceMessageId = field(approval ?? {}, 'message_id') || approvalId;

  if (action === 'declined') {
    await updateApproval(client, approvalId, 'declined');
    await insertAssistantMessage(client, {
      conversationId,
      replyToMessageId: sourceMessageId,
      body: 'Approval was declined, so I stopped the action and left the workspace unchanged.',
      status: 'complete',
    });
    return;
  }

  await updateApproval(client, approvalId, 'approved');
  await streamThenInsertReply(client, conversationId, sourceMessageId, buildAssistantReply('approval granted'));
}

export async function startReactAiChatAgent(stopSignal?: AbortSignal): Promise<void> {
  const client = createConsumerClient({
    url: KALAMDB_URL,
    authProvider: async () => Auth.basic(KALAMDB_USER, KALAMDB_PASSWORD),
  });
  const sqlClient = client as unknown as KalamDBClient;

  await Promise.all([
    runAgent<TopicRow>({
      client,
      name: 'react-ai-chat-message-agent',
      topic: MESSAGE_TOPIC,
      groupId: process.env.KALAMDB_GROUP ?? 'react-ai-chat-message-agent',
      start: 'earliest',
      batchSize: 10,
      timeoutSeconds: 30,
      stopSignal,
      onRow: async (_ctx, row) => handleUserMessage(sqlClient, row),
    }),
    runAgent<TopicRow>({
      client,
      name: 'react-ai-chat-action-agent',
      topic: ACTION_TOPIC,
      groupId: process.env.KALAMDB_ACTION_GROUP ?? 'react-ai-chat-action-agent',
      start: 'earliest',
      batchSize: 10,
      timeoutSeconds: 30,
      stopSignal,
      onRow: async (_ctx, row) => handleApprovalAction(sqlClient, row),
    }),
  ]);
}

function cellString(value: unknown): string {
  if (value && typeof value === 'object' && 'asString' in value && typeof value.asString === 'function') {
    return value.asString() ?? '';
  }
  return value == null ? '' : String(value);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const controller = new AbortController();
  process.on('SIGINT', () => controller.abort());
  process.on('SIGTERM', () => controller.abort());

  startReactAiChatAgent(controller.signal).catch((error) => {
    console.error('react-ai-chat-agent failed', error);
    process.exit(1);
  });
}