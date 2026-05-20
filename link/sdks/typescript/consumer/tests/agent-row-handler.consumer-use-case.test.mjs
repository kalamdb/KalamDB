import assert from 'node:assert/strict';
import test from 'node:test';

import { runAgent } from '../dist/src/index.js';
import { createConsumerScenarioClient, makeConsumerMessage } from './use-case-helpers.mjs';

test('consumer compatibility: runAgent alias still supports onRow for AI reply workers', async () => {
  const message = makeConsumerMessage({
    offset: 91,
    topic: 'ai_chat.reply_events',
    group_id: 'reply-materializer',
    user: 'assistant-service',
    op: 'Insert',
    payload: { id: 'reply-91', conversation_id: 'conv-91', body: 'Here is the answer.', role: 'assistant' },
  });
  const { client, state } = createConsumerScenarioClient([message]);
  const rows = [];

  await runAgent({
    client,
    name: 'reply-materializer',
    topic: 'ai_chat.reply_events',
    groupId: 'reply-materializer',
    retry: { maxAttempts: 1, initialBackoffMs: 0, maxBackoffMs: 0 },
    onRow: async (ctx, row, change) => {
      rows.push({ rowId: row.id, offset: change.offset, attempt: ctx.attempt });
      await ctx.sql(
        'INSERT INTO ai_chat.messages (id, conversation_id, role, body) VALUES ($1, $2, $3, $4)',
        [row.id, row.conversation_id, row.role, row.body],
      );
    },
  });

  assert.deepEqual(rows, [{ rowId: 'reply-91', offset: 91, attempt: 1 }]);
  assert.deepEqual(state.queryCalls[0].params, ['reply-91', 'conv-91', 'assistant', 'Here is the answer.']);
  assert.deepEqual(state.ackedOffsets, [91]);
});