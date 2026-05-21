import assert from 'node:assert/strict';
import test from 'node:test';

import { runConsumer } from '../dist/src/index.js';
import { createConsumerScenarioClient, makeConsumerMessage } from './use-case-helpers.mjs';

test('consumer use case 06: job status topic applies legal state transitions in order', async () => {
  const messages = [
    makeConsumerMessage({ offset: 51, topic: 'jobs.status_events', group_id: 'job-state-worker', user: 'system', op: 'Insert', payload: { id: 'job-1', status: 'queued', priority: 'high' } }),
    makeConsumerMessage({ offset: 52, topic: 'jobs.status_events', group_id: 'job-state-worker', user: 'worker-a', op: 'Update', payload: { id: 'job-1', status: 'running', priority: 'high' } }),
    makeConsumerMessage({ offset: 53, topic: 'jobs.status_events', group_id: 'job-state-worker', user: 'worker-a', op: 'Update', payload: { id: 'job-1', status: 'completed', priority: 'high' } }),
  ];
  const { client, state } = createConsumerScenarioClient(messages);
  const statusByJob = new Map();
  const allowed = new Set(['undefined->queued', 'queued->running', 'running->completed']);

  await runConsumer({
    client,
    name: 'job-state-worker',
    topic: 'jobs.status_events',
    groupId: 'job-state-worker',
    retry: { maxAttempts: 1, initialBackoffMs: 0, maxBackoffMs: 0 },
    onChange: async (ctx, change) => {
      const previous = statusByJob.get(change.data.id);
      const transition = `${previous}->${change.data.status}`;
      assert.ok(allowed.has(transition), `unexpected transition ${transition}`);
      statusByJob.set(change.data.id, change.data.status);
      await ctx.sql(
        'INSERT INTO jobs.status_audit (job_id, previous_status, next_status, actor) VALUES ($1, $2, $3, $4)',
        [change.data.id, previous ?? null, change.data.status, change.user],
      );
    },
  });

  assert.equal(statusByJob.get('job-1'), 'completed');
  assert.deepEqual(state.queryCalls.map((call) => call.params), [
    ['job-1', null, 'queued', 'system'],
    ['job-1', 'queued', 'running', 'worker-a'],
    ['job-1', 'running', 'completed', 'worker-a'],
  ]);
  assert.deepEqual(state.ackedOffsets, [51, 52, 53]);
});