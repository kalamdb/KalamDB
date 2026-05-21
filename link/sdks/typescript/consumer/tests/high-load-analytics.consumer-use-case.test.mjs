import assert from 'node:assert/strict';
import test from 'node:test';

import { runConsumer } from '../dist/src/index.js';
import { createConsumerScenarioClient, makeConsumerMessage, sortNumbers } from './use-case-helpers.mjs';

test('consumer use case 08: high-load analytics worker handles hundreds of concurrent message events', async () => {
  const messages = Array.from({ length: 500 }, (_unused, index) => makeConsumerMessage({
    offset: 1_000 + index,
    topic: 'analytics.chat_events',
    group_id: 'analytics-rollup',
    user: `user-${index % 25}`,
    op: 'Insert',
    payload: {
      id: `event-${index}`,
      room_id: `room-${index % 10}`,
      event_name: index % 3 === 0 ? 'message_sent' : 'message_read',
      latency_ms: 20 + (index % 17),
    },
  }));
  const { client, state } = createConsumerScenarioClient(messages, { parallel: true });
  const counts = new Map();

  await runConsumer({
    client,
    name: 'analytics-rollup',
    topic: 'analytics.chat_events',
    groupId: 'analytics-rollup',
    batchSize: 500,
    retry: { maxAttempts: 1, initialBackoffMs: 0, maxBackoffMs: 0 },
    onChange: async (_ctx, change) => {
      const key = `${change.data.room_id}:${change.data.event_name}`;
      counts.set(key, (counts.get(key) ?? 0) + 1);
    },
  });

  assert.equal([...counts.values()].reduce((sum, value) => sum + value, 0), 500);
  assert.equal(state.ackedOffsets.length, 500);
  assert.deepEqual(sortNumbers(state.ackedOffsets).slice(0, 3), [1000, 1001, 1002]);
  assert.deepEqual(sortNumbers(state.ackedOffsets).slice(-3), [1497, 1498, 1499]);
});