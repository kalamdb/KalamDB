import assert from 'node:assert/strict';
import test from 'node:test';

import { runConsumer } from '../dist/src/index.js';
import { createConsumerScenarioClient, makeConsumerMessage } from './use-case-helpers.mjs';

test('consumer use case 05: chat delete and edit events redact search indexes', async () => {
  const messages = [
    makeConsumerMessage({ offset: 41, topic: 'chat.message_cdc', group_id: 'search-redactor', user: 'alice', op: 'Update', payload: { id: 'm41', body: '[removed]', redacted: true } }),
    makeConsumerMessage({ offset: 42, topic: 'chat.message_cdc', group_id: 'search-redactor', user: 'moderator', op: 'Delete', payload: { id: 'm42', _deleted: true } }),
    makeConsumerMessage({ offset: 43, topic: 'chat.message_cdc', group_id: 'search-redactor', user: 'bob', op: 'Update', payload: { id: 'm43', body: 'normal edit', redacted: false } }),
  ];
  const { client, state } = createConsumerScenarioClient(messages);

  await runConsumer({
    client,
    name: 'search-redactor',
    topic: 'chat.message_cdc',
    groupId: 'search-redactor',
    retry: { maxAttempts: 1, initialBackoffMs: 0, maxBackoffMs: 0 },
    onChange: async (ctx, change) => {
      if (change.op === 'Delete' || change.data.redacted === true) {
        await ctx.sql('DELETE FROM chat.search_index WHERE message_id = $1', [change.data.id]);
        return;
      }
      await ctx.sql('UPDATE chat.search_index SET body = $2 WHERE message_id = $1', [change.data.id, change.data.body]);
    },
  });

  assert.deepEqual(state.queryCalls.map((call) => ({ sql: call.sql, params: call.params })), [
    { sql: 'DELETE FROM chat.search_index WHERE message_id = $1', params: ['m41'] },
    { sql: 'DELETE FROM chat.search_index WHERE message_id = $1', params: ['m42'] },
    { sql: 'UPDATE chat.search_index SET body = $2 WHERE message_id = $1', params: ['m43', 'normal edit'] },
  ]);
  assert.deepEqual(state.ackedOffsets, [41, 42, 43]);
});