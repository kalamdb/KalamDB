import { config as loadEnv } from 'dotenv';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Auth } from '@kalamdb/client';
import { ConsumerChange, ConsumerRunContext, createConsumerClient, runConsumer } from '@kalamdb/consumer';
import { executeAsUser, kalamDriver } from '@kalamdb/orm';
import { drizzle } from 'drizzle-orm/pg-proxy';
import {
  chat_demo_agent_events as agentEvents,
  chat_demo_agent_eventsConfig as agentEventsConfig,
  chat_demo_messages as chatMessages,
  chat_demo_messagesConfig as chatMessagesConfig,
  type ChatDemoMessages as ChatMessageRow,
} from './schema.generated.js';

const __dirname = dirname(fileURLToPath(import.meta.url));
loadEnv({ path: resolve(__dirname, '../.env.local'), quiet: true });
loadEnv({ path: resolve(__dirname, '../.env'), quiet: true });

const KALAMDB_URL = process.env.KALAMDB_URL ?? 'http://127.0.0.1:2900';
const KALAMDB_USER = process.env.KALAMDB_USER ?? 'root';
const KALAMDB_PASSWORD = process.env.KALAMDB_PASSWORD ?? 'kalamdb123';
const THINKING_DELAY_MS = 250;
const STREAM_DELAY_MS = 120;
const STREAM_CHUNK_SIZE = 64;

const TOPIC_NAME = 'chat_demo.ai_inbox';
const CONSUMER_GROUP = process.env.KALAMDB_GROUP ?? 'chat-ai-agent';
const CONSUMER_START = (process.env.KALAMDB_START ?? 'latest').trim().toLowerCase() === 'earliest'
  ? 'earliest'
  : 'latest';

type StartAgentOptions = {
  stopSignal?: AbortSignal;
};

type AgentEventStage = 'thinking' | 'typing' | 'message_saved' | 'complete' | 'log';

async function sleep(ms: number): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, ms));
}

function assertValidUser(user: string): string {
  if (!/^[A-Za-z0-9._-]+$/.test(user)) {
    throw new Error(`Unsupported user for EXECUTE AS USER: ${user}`);
  }

  return user;
}

export function buildReply(content: string): string {
  const trimmed = content.trim();
  const lowered = trimmed.toLowerCase();
  let extra = 'This demo now uses runConsumer() to consume new user messages from a topic — zero polling, zero wasted queries.';

  if (lowered.includes('latency')) {
    extra = 'I would inspect the slowest route first, then compare the latest write volume against the baseline you see in the database.';
  } else if (lowered.includes('deploy')) {
    extra = 'That sounds deploy-shaped. I would compare the newest release marker against the first spike in user messages.';
  } else if (lowered.includes('queue')) {
    extra = 'Queue growth usually means a downstream dependency is flattening. This is a good fit for another agent wired to a different topic.';
  }

  return `AI reply: KalamDB stored "${trimmed}" in chat_demo.messages, streamed the drafting state through chat_demo.agent_events, and committed the final reply with EXECUTE AS USER. ${extra}`;
}

export async function startChatAgent(options: StartAgentOptions = {}): Promise<void> {
  const client = createConsumerClient({
    url: KALAMDB_URL,
    authProvider: async () => Auth.basic(KALAMDB_USER, KALAMDB_PASSWORD),
  });
  const db = drizzle(kalamDriver(client));
  const canReadMessageSeq = chatMessagesConfig.systemColumns.includes('_seq');

  const emitEvent = async (
    room: string,
    senderUsername: string,
    responseId: string,
    stage: AgentEventStage,
    preview: string,
    message: string,
  ): Promise<void> => {
    await executeAsUser(
      client,
      db.insert(agentEvents).values({
        response_id: responseId,
        room,
        sender_username: senderUsername,
        stage,
        preview,
        message,
      }),
      assertValidUser(senderUsername),
    );
  };

  const insertAssistantMessage = async (
    room: string,
    senderUsername: string,
    reply: string,
  ): Promise<void> => {
    await executeAsUser(
      client,
      db.insert(chatMessages).values({
        room,
        role: 'assistant',
        author: 'KalamDB Copilot',
        sender_username: senderUsername,
        content: reply,
      }),
      assertValidUser(senderUsername),
    );
  };

  console.log(`[chat-demo-agent] starting (url=${KALAMDB_URL}, user=${KALAMDB_USER}, mode=topic-consumer)`);
  console.log(`[chat-demo-agent] topic=${TOPIC_NAME}  group=${CONSUMER_GROUP}  start=${CONSUMER_START}`);
  console.log(`[chat-demo-agent] messages=${chatMessagesConfig.tableType} events=${agentEventsConfig.tableType} seq=${canReadMessageSeq ? 'enabled' : 'disabled'}`);
  console.log(`[chat-demo-agent] connecting to KalamDB at ${KALAMDB_URL} ...`);

  let awaitingReconnect = false;

  await runConsumer<ChatMessageRow>({
    client,
    name: 'chat-ai-agent',
    topic: TOPIC_NAME,
    groupId: CONSUMER_GROUP,
    start: CONSUMER_START,
    batchSize: 10,
    timeoutSeconds: 30,
    stopSignal: options.stopSignal,

    onChange: async (_ctx: ConsumerRunContext<ChatMessageRow>, change: ConsumerChange<ChatMessageRow>) => {
      const row = change.data as ChatMessageRow;
      if (row.role !== 'user') {
        console.log(`[agent] skipping non-user message id=${row.id} role=${row.role}`);
        return;
      }

      const responseId = `reply-${row.id}`;

      const seqLabel = canReadMessageSeq && row._seq ? ` seq=${row._seq}` : '';
      console.log(`[agent] received user message id=${row.id}${seqLabel} user=${row.sender_username} room=${row.room}`);
      await emitEvent(row.room, row.sender_username, responseId, 'log', '', `Picked up user message ${row.id}`);
      await emitEvent(row.room, row.sender_username, responseId, 'thinking', '', 'Planning assistant reply');
      await sleep(THINKING_DELAY_MS);

      const reply = buildReply(row.content);
      let streamedReply = '';

      for (let index = 0; index < reply.length; index += STREAM_CHUNK_SIZE) {
        streamedReply += reply.slice(index, index + STREAM_CHUNK_SIZE);
        await emitEvent(row.room, row.sender_username, responseId, 'typing', streamedReply, `Streamed ${streamedReply.length}/${reply.length} characters`);
        await sleep(STREAM_DELAY_MS);
      }

      console.log(`[agent] committing assistant reply for user=${row.sender_username} message=${row.id}`);
      await emitEvent(row.room, row.sender_username, responseId, 'log', streamedReply, 'Persisting final assistant reply');
      await insertAssistantMessage(row.room, row.sender_username, reply);
      await emitEvent(row.room, row.sender_username, responseId, 'message_saved', reply, 'Assistant reply committed');
      await sleep(STREAM_DELAY_MS);
      await emitEvent(row.room, row.sender_username, responseId, 'complete', reply, 'Live stream finished');
      console.log(`[agent] completed reply for user=${row.sender_username} message=${row.id}`);
    },

    onError: ({ error, runKey }) => {
      console.error(`[chat-demo-agent] error processing ${runKey}:`, error);
    },

    onRetry: ({ attempt, maxAttempts, backoffMs, runKey }) => {
      console.warn(`[chat-demo-agent] retrying ${runKey} (attempt ${attempt}/${maxAttempts}, backoff ${backoffMs}ms)`);
    },

    onConnectionRetry: ({ error, attempt, maxAttempts, backoffMs }) => {
      if (!awaitingReconnect) {
        console.warn(`[chat-demo-agent] cannot reach KalamDB at ${KALAMDB_URL}: ${error instanceof Error ? error.message : String(error)}`);
        awaitingReconnect = true;
      }
      const attemptLabel = maxAttempts ? `${attempt}/${maxAttempts}` : `${attempt}`;
      console.warn(`[chat-demo-agent] reconnecting in ${backoffMs}ms (attempt ${attemptLabel})`);
    },

    onConnectionRestored: ({ attempt }) => {
      console.log(`[chat-demo-agent] connected to KalamDB after ${attempt} reconnect ${attempt === 1 ? 'attempt' : 'attempts'}`);
      awaitingReconnect = false;
    },

    onConnectionError: ({ error, attempt }) => {
      console.error(`[chat-demo-agent] gave up reconnecting after ${attempt} ${attempt === 1 ? 'attempt' : 'attempts'}: ${error instanceof Error ? error.message : String(error)}`);
      awaitingReconnect = false;
    },
  });
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const controller = new AbortController();
  process.on('SIGINT', () => controller.abort());
  process.on('SIGTERM', () => controller.abort());

  startChatAgent({ stopSignal: controller.signal }).catch((error) => {
    console.error('chat-demo-agent failed', error);
    process.exit(1);
  });
}
