import { describe, it, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { Auth } from '@kalamdb/client';
import { createConsumerClient, runConsumer } from '@kalamdb/consumer';
import { drizzle } from 'drizzle-orm/pg-proxy';
import { and, eq } from 'drizzle-orm';
import { text } from 'drizzle-orm/pg-core';
import { executeAsUser, kalamDriver, kTable } from '../dist/index.js';
import { requirePassword, createTestClient, URL, USER, PASS } from './helpers.mjs';

requirePassword();

const namespace = `test_agent_${Date.now()}_${randomUUID().replace(/-/g, '').slice(0, 8)}`;
const inboxTableName = `${namespace}.inbox`;
const processedTableName = `${namespace}.processed`;
const topicName = `${namespace}.agent_inbox`;

let adminClient;
let workerClient;
let workerDb;

const inbox = kTable.stream(inboxTableName, {
  id: text('id'),
  kind: text('kind'),
  bucket: text('bucket'),
  body: text('body'),
});

const processed = kTable.user(processedTableName, {
  id: text('id'),
  source_id: text('source_id'),
  kind: text('kind'),
  bucket: text('bucket'),
  summary: text('summary'),
});

function sqlLiteral(value) {
  return `'${String(value).replace(/'/g, "''")}'`;
}

async function waitFor(condition, timeoutMs = 20_000, intervalMs = 100) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const value = await condition();
    if (value) return value;
    await new Promise((resolve) => setTimeout(resolve, intervalMs));
  }
  throw new Error(`Timed out after ${timeoutMs}ms`);
}

async function waitForTopicRoute() {
  await waitFor(async () => {
    const row = await adminClient.queryOne(
      `SELECT routes FROM system.topics WHERE topic_id = ${sqlLiteral(topicName)}`,
    );
    const raw = row?.routes?.asString?.();
    if (!raw) return false;
    try {
      const routes = JSON.parse(raw);
      return Array.isArray(routes) && routes.length > 0;
    } catch {
      return false;
    }
  }, 30_000, 100);
}

before(async () => {
  adminClient = createTestClient();
  await adminClient.initialize();
  await adminClient.login();

  workerClient = createConsumerClient({
    url: URL,
    authProvider: async () => Auth.basic(USER, PASS),
  });
  workerDb = drizzle(kalamDriver(workerClient));

  await adminClient.query(`CREATE NAMESPACE IF NOT EXISTS ${namespace}`);
  await adminClient.query(`
    CREATE TABLE ${inboxTableName} (
      id TEXT PRIMARY KEY,
      kind TEXT NOT NULL,
      bucket TEXT NOT NULL,
      body TEXT NOT NULL
    )
  `);
  await adminClient.query(`
    CREATE TABLE ${processedTableName} (
      id TEXT PRIMARY KEY,
      source_id TEXT NOT NULL,
      kind TEXT NOT NULL,
      bucket TEXT NOT NULL,
      summary TEXT NOT NULL
    ) WITH (TYPE = 'USER')
  `);
  await adminClient.query(`CREATE TOPIC ${topicName}`);
  await adminClient.query(`ALTER TOPIC ${topicName} ADD SOURCE ${inboxTableName} ON INSERT`);
  await waitForTopicRoute();
});

after(async () => {
  await workerClient?.disconnect?.().catch(() => {});
  await adminClient?.query(`DROP TOPIC IF EXISTS ${topicName}`).catch(() => {});
  await adminClient?.query(`DROP TABLE IF EXISTS ${processedTableName}`).catch(() => {});
  await adminClient?.query(`DROP TABLE IF EXISTS ${inboxTableName}`).catch(() => {});
  await adminClient?.query(`DROP NAMESPACE IF EXISTS ${namespace}`).catch(() => {});
  await adminClient?.disconnect();
});

describe('ORM with @kalamdb/consumer runConsumer', () => {
  it('uses generated-style tables, filters rows, and writes through ORM builders in a consumer worker', async () => {
    const groupId = `orm-agent-${randomUUID()}`;
    const targetBucket = `bucket-${randomUUID()}`;
    const controller = new AbortController();

    const agentTask = runConsumer({
      client: workerClient,
      name: 'orm-consumer-agent',
      topic: topicName,
      groupId,
      start: 'earliest',
      batchSize: 2,
      timeoutSeconds: 2,
      stopSignal: controller.signal,
      retry: {
        maxAttempts: 2,
        initialBackoffMs: 0,
        maxBackoffMs: 0,
      },
      onChange: async (_ctx, change) => {
        const row = change.data;
        if (row.kind !== 'summarize' || row.bucket !== targetBucket) {
          return;
        }

        await executeAsUser(
          workerClient,
          workerDb.insert(processed).values({
            id: `processed-${row.id}`,
            source_id: String(row.id),
            kind: String(row.kind),
            bucket: String(row.bucket),
            summary: `summary:${String(row.body).toUpperCase()}`,
          }),
          USER,
        );

        const matchingRows = await workerDb
          .select({ id: processed.id, source_id: processed.source_id })
          .from(processed)
          .where(and(eq(processed.bucket, targetBucket), eq(processed.kind, 'summarize')));
        assert.ok(matchingRows.some((item) => item.source_id === row.id));
      },
    });

    try {
      await new Promise((resolve) => setTimeout(resolve, 500));
      await adminClient.query([
        `INSERT INTO ${inboxTableName} (id, kind, bucket, body) VALUES`,
        `('skip-kind', 'ignore', ${sqlLiteral(targetBucket)}, 'ignore me'),`,
        `('skip-bucket', 'summarize', 'other-bucket', 'wrong bucket'),`,
        `('work-1', 'summarize', ${sqlLiteral(targetBucket)}, 'first job'),`,
        `('work-2', 'summarize', ${sqlLiteral(targetBucket)}, 'second job')`,
      ].join(' '));

      const sourceRows = await workerDb
        .select({ id: inbox.id })
        .from(inbox)
        .where(and(eq(inbox.bucket, targetBucket), eq(inbox.kind, 'summarize')));
      assert.deepEqual(sourceRows.map((row) => row.id).sort(), ['work-1', 'work-2']);

      const rows = await waitFor(async () => {
        const found = await workerDb
          .select({ source_id: processed.source_id, summary: processed.summary })
          .from(processed)
          .where(and(eq(processed.bucket, targetBucket), eq(processed.kind, 'summarize')));
        return found.length === 2 ? found : false;
      });

      const summaries = new Map(rows.map((row) => [row.source_id, row.summary]));
      assert.equal(summaries.get('work-1'), 'summary:FIRST JOB');
      assert.equal(summaries.get('work-2'), 'summary:SECOND JOB');
      assert.equal(summaries.has('skip-kind'), false);
      assert.equal(summaries.has('skip-bucket'), false);
    } finally {
      controller.abort();
      await Promise.race([
        agentTask,
        new Promise((resolve) => setTimeout(resolve, 3_000)),
      ]);
    }
  });
});
