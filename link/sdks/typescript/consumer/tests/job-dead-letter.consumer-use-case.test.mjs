import assert from 'node:assert/strict';
import test from 'node:test';

import { runConsumer } from '../dist/src/index.js';
import { createConsumerScenarioClient, makeConsumerMessage } from './use-case-helpers.mjs';

test('consumer use case 07: job worker retries transient failures then writes a dead-letter row', async () => {
  const message = makeConsumerMessage({
    offset: 61,
    topic: 'jobs.dispatch_events',
    group_id: 'email-job-worker',
    user: 'scheduler',
    op: 'Insert',
    payload: { id: 'job-email-1', queue: 'email', payload_json: '{"to":"a@example.com"}' },
  });
  const { client, state } = createConsumerScenarioClient([message]);
  const retryAttempts = [];
  let failedContext = null;

  await runConsumer({
    client,
    name: 'email-job-worker',
    topic: 'jobs.dispatch_events',
    groupId: 'email-job-worker',
    retry: { maxAttempts: 3, initialBackoffMs: 0, maxBackoffMs: 0 },
    onChange: async () => {
      throw new Error('smtp provider unavailable');
    },
    onRetry: (event) => retryAttempts.push(event.attempt),
    onFailed: async (ctx, change) => {
      failedContext = { attempt: ctx.attempt, runKey: ctx.runKey, error: String(ctx.error) };
      await ctx.sql(
        'INSERT INTO jobs.dead_letters (job_id, run_key, error) VALUES ($1, $2, $3)',
        [change.data.id, ctx.runKey, String(ctx.error)],
      );
    },
    ackOnFailed: true,
  });

  assert.deepEqual(retryAttempts, [1, 2]);
  assert.deepEqual(failedContext, {
    attempt: 3,
    runKey: 'email-job-worker:jobs.dispatch_events:0:61',
    error: 'Error: smtp provider unavailable',
  });
  assert.deepEqual(state.queryCalls[0].params, [
    'job-email-1',
    'email-job-worker:jobs.dispatch_events:0:61',
    'Error: smtp provider unavailable',
  ]);
  assert.deepEqual(state.ackedOffsets, [61]);
});