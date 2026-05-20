import assert from 'node:assert/strict';
import test from 'node:test';

import { runConsumer } from '../dist/src/index.js';
import { createConsumerScenarioClient, makeConsumerMessage } from './use-case-helpers.mjs';

test('consumer use case 09: webhook outbox uses run keys to skip duplicate deliveries', async () => {
  const messages = [
    makeConsumerMessage({ offset: 71, topic: 'integrations.webhook_events', group_id: 'webhook-dispatcher', key: 'tenant-a:webhook-1', user: 'tenant-a', payload: { id: 'webhook-1', target_url: 'https://hooks.example/a', body: '{"ok":true}' } }),
    makeConsumerMessage({ offset: 72, topic: 'integrations.webhook_events', group_id: 'webhook-dispatcher', key: 'tenant-a:webhook-1', user: 'tenant-a', payload: { id: 'webhook-1', target_url: 'https://hooks.example/a', body: '{"ok":true}' } }),
    makeConsumerMessage({ offset: 73, topic: 'integrations.webhook_events', group_id: 'webhook-dispatcher', key: 'tenant-b:webhook-2', user: 'tenant-b', payload: { id: 'webhook-2', target_url: 'https://hooks.example/b', body: '{"ok":true}' } }),
  ];
  const delivered = new Set();
  const { client, state } = createConsumerScenarioClient(messages, {
    queryOne: async (_sql, params) => delivered.has(params[0]) ? { run_key: params[0] } : null,
  });

  await runConsumer({
    client,
    name: 'webhook-dispatcher',
    topic: 'integrations.webhook_events',
    groupId: 'webhook-dispatcher',
    runKeyFactory: ({ name, message }) => `${name}:${message.key}`,
    retry: { maxAttempts: 1, initialBackoffMs: 0, maxBackoffMs: 0 },
    onChange: async (ctx, change) => {
      const existing = await ctx.queryOne('SELECT run_key FROM integrations.webhook_outbox WHERE run_key = $1', [ctx.runKey]);
      if (existing) return;
      delivered.add(ctx.runKey);
      await ctx.sql(
        'INSERT INTO integrations.webhook_outbox (run_key, target_url, body) VALUES ($1, $2, $3)',
        [ctx.runKey, change.data.target_url, change.data.body],
      );
    },
  });

  assert.deepEqual(state.queryOneCalls.map((call) => call.params[0]), [
    'webhook-dispatcher:tenant-a:webhook-1',
    'webhook-dispatcher:tenant-a:webhook-1',
    'webhook-dispatcher:tenant-b:webhook-2',
  ]);
  assert.deepEqual(state.queryCalls.map((call) => call.params[0]), [
    'webhook-dispatcher:tenant-a:webhook-1',
    'webhook-dispatcher:tenant-b:webhook-2',
  ]);
  assert.deepEqual(state.ackedOffsets, [71, 72, 73]);
});