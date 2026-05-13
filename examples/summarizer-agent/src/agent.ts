import { config as loadEnv } from 'dotenv';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Auth } from '@kalamdb/client';

const __dirname = dirname(fileURLToPath(import.meta.url));
import { createConsumerClient, runConsumer } from '@kalamdb/consumer';

type StartAgentOptions = {
  stopSignal?: AbortSignal;
  groupId?: string;
  start?: 'latest' | 'earliest';
};

type AgentConfig = {
  url: string;
  user: string;
  password: string;
  topic: string;
  group: string;
};

function readConfig(): AgentConfig {
  loadEnv({ path: resolve(__dirname, '../.env.local'), quiet: true });
  loadEnv({ path: resolve(__dirname, '../.env'), quiet: true });

  return {
    url: process.env.KALAMDB_URL ?? 'http://127.0.0.1:2900',
    user: process.env.KALAMDB_USER ?? 'root',
    password: process.env.KALAMDB_PASSWORD ?? 'kalamdb123',
    topic: process.env.KALAMDB_TOPIC ?? 'blog.summarizer',
    group: process.env.KALAMDB_GROUP ?? 'blog-summarizer-agent',
  };
}

function formatErrorMessage(error: unknown): string {
  if (error instanceof Error && error.message) {
    return error.message;
  }

  return String(error);
}

export function buildSummary(content: string): string {
  const compact = content.replace(/\s+/g, ' ').trim();
  const sentence = compact.split(/[.!?]/)[0]?.trim() ?? compact;
  const shortened = sentence.slice(0, 140).trim();
  let summary = shortened.endsWith('.') ? shortened : `${shortened}.`;
  summary += ' This blog post is about ';
  const keywords = ['technology', 'health', 'finance', 'travel', 'food', 'education'];
  let hash = 0;
  for (let index = 0; index < compact.length; index += 1) {
    hash = (hash * 31 + compact.charCodeAt(index)) >>> 0;
  }
  const keyword = keywords[hash % keywords.length];
  summary += keyword + '.';
  return summary;
}

export async function startSummarizerAgent(options: StartAgentOptions = {}): Promise<void> {
  const config = readConfig();
  const client = createConsumerClient({
    url: config.url,
    authProvider: async () => Auth.basic(config.user, config.password),
  });

  const groupId = options.groupId ?? config.group;
  const start = options.start ?? 'latest';
  let awaitingReconnect = false;

  console.log(`[summarizer-agent] starting (url=${config.url}, topic=${config.topic}, group=${groupId}, start=${start})`);
  console.log(`[summarizer-agent] connecting to KalamDB at ${config.url} ...`);

  try {
    await runConsumer<Record<string, unknown>>({
      client,
      name: 'summarizer-agent',
      topic: config.topic,
      groupId,
      start,
      stopSignal: options.stopSignal,
      retry: { maxAttempts: 3, initialBackoffMs: 250, maxBackoffMs: 1500 },
      onConnectionRetry: ({ error, attempt, maxAttempts, backoffMs }) => {
        if (!awaitingReconnect) {
          console.warn(`[summarizer-agent] cannot reach KalamDB at ${config.url}: ${formatErrorMessage(error)}`);
          awaitingReconnect = true;
        }

        const attemptLabel = maxAttempts ? `${attempt}/${maxAttempts}` : `${attempt}`;
        console.warn(
          `[summarizer-agent] reconnecting in ${backoffMs}ms (attempt ${attemptLabel})`,
        );
      },
      onConnectionRestored: ({ attempt }) => {
        const attemptsLabel = attempt === 1 ? 'attempt' : 'attempts';
        console.log(`[summarizer-agent] connected to KalamDB after ${attempt} reconnect ${attemptsLabel}`);
        awaitingReconnect = false;
      },
      onConnectionError: ({ error, attempt }) => {
        const attemptsLabel = attempt === 1 ? 'attempt' : 'attempts';
        console.error(
          `summarizer-agent stopped reconnecting after ${attempt} ${attemptsLabel}: ${formatErrorMessage(error)}`,
        );
        awaitingReconnect = false;
      },
      onChange: async (ctx, change) => {
        const row = change.data;
        const blogId = row.blog_id ?? row.blogId;
        const content = String(row.content ?? '').trim();
        const currentSummary = String(row.summary ?? '').trim();

        if (!blogId || !content) {
          return;
        }

        const nextSummary = buildSummary(content);
        if (currentSummary === nextSummary) {
          return;
        }

        await ctx.sql(
          'UPDATE blog.blogs SET summary = $1, updated = NOW() WHERE blog_id = $2',
          [nextSummary, blogId],
        );
        console.log(`[summarizer-agent] updated summary for blog_id=${blogId} (summary="${nextSummary}")`);
      },
      onFailed: async (ctx, change) => {
        await ctx.sql(
          'INSERT INTO blog.summary_failures (run_key, blog_id, error) VALUES ($1, $2, $3)',
          [ctx.runKey, String(change.data.blog_id ?? 'unknown'), String(ctx.error ?? 'unknown')],
        );
      },
      ackOnFailed: true,
      onError: ({ error }) => {
        console.error('[summarizer-agent] processing error:', error);
      },
    });
  } finally {
    console.log('[summarizer-agent] disconnecting');
    await client.disconnect();
  }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const controller = new AbortController();
  process.on('SIGINT', () => controller.abort());
  process.on('SIGTERM', () => controller.abort());

  startSummarizerAgent({ stopSignal: controller.signal }).catch((error) => {
    console.error('summarizer-agent failed:', error);
    process.exit(1);
  });
}
