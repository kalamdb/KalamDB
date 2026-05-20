import assert from 'node:assert/strict';
import test from 'node:test';

import { runConsumer } from '@kalamdb/consumer';
import { drizzle } from 'drizzle-orm/pg-proxy';
import { eq } from 'drizzle-orm';
import { integer, text, timestamp } from 'drizzle-orm/pg-core';
import { executeAsUser, generateSchema, kalamDriver, kTable } from '../dist/index.js';
import { column, createOrmConsumerClient, createSchemaClient, makeOrmMessage, tableInfo } from './orm-consumer-use-case-helpers.mjs';

test('ORM use case 03: job dispatch tables generate and consumer reserves worker capacity through ORM updates', async () => {
  const jobs = kTable.shared('jobs.jobs', {
    id: text('id').primaryKey(),
    queue: text('queue').notNull(),
    status: text('status').notNull(),
    priority: integer('priority').notNull(),
  });
  const runs = kTable.user('jobs.runs', {
    id: text('id').primaryKey(),
    job_id: text('job_id').notNull(),
    worker_id: text('worker_id').notNull(),
    started_at: timestamp('started_at', { mode: 'string' }),
  });
  const events = kTable.stream('jobs.job_events', {
    id: text('id').primaryKey(),
    job_id: text('job_id').notNull(),
    status: text('status').notNull(),
  });

  const schema = await generateSchema(createSchemaClient([
    tableInfo('jobs', 'jobs', 'Shared', [column('id', 'Text', { ordinal: 1, primary: true, nullable: false }), column('priority', 'Int', { ordinal: 2, nullable: false })]),
    tableInfo('jobs', 'runs', 'User', [column('id', 'Text', { ordinal: 1, primary: true, nullable: false }), column('started_at', 'Timestamp', { ordinal: 2 })]),
    tableInfo('jobs', 'job_events', 'Stream', [column('id', 'Text', { ordinal: 1, primary: true, nullable: false }), column('status', 'Text', { ordinal: 2, nullable: false })]),
  ]));
  assert.ok(schema.includes('integer("priority").notNull()'));
  assert.ok(schema.includes('timestamp("started_at", { mode: \'date\' })'));

  const topicName = 'jobs.dispatch_topic';
  const { client, state } = createOrmConsumerClient([
    makeOrmMessage({ offset: 20, topic: topicName, group_id: 'job-dispatch-worker', user: 'scheduler', op: 'Insert', payload: { id: 'job-20', queue: 'emails', priority: 10 } }),
  ], {
    queryOne: async () => ({ worker_id: 'worker-a', available_slots: 1 }),
  });
  const db = drizzle(kalamDriver(client));

  await runConsumer({
    client,
    name: 'job-dispatch-worker',
    topic: topicName,
    groupId: 'job-dispatch-worker',
    retry: { maxAttempts: 1, initialBackoffMs: 0, maxBackoffMs: 0 },
    onChange: async (ctx, change) => {
      const worker = await ctx.queryOne('SELECT worker_id, available_slots FROM jobs.worker_capacity WHERE queue = $1', [change.data.queue]);
      assert.equal(worker.worker_id, 'worker-a');
      await executeAsUser(client, db.update(jobs).set({ status: 'running' }).where(eq(jobs.id, String(change.data.id))), 'system');
      await executeAsUser(client, db.insert(runs).values({ id: `run-${change.data.id}`, job_id: String(change.data.id), worker_id: worker.worker_id, started_at: '2026-05-20T12:00:00' }), 'system');
    },
  });

  assert.equal(events.id.name, 'id');
  assert.deepEqual(state.queryOneCalls[0].params, ['emails']);
  assert.match(state.executeAsUserCalls[0].sql, /update jobs\.jobs set/i);
  assert.match(state.executeAsUserCalls[1].sql, /insert into jobs\.runs/i);
  assert.deepEqual(state.ackedOffsets, [20]);
});