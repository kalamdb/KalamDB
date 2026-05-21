import assert from 'node:assert/strict';
import test from 'node:test';

import { runConsumer } from '../dist/src/index.js';
import { createConsumerScenarioClient, makeConsumerMessage, sortNumbers } from './use-case-helpers.mjs';

test('consumer use case 04: concurrent AI tool-call events keep each handler context isolated', async () => {
  const messages = [
    makeConsumerMessage({ offset: 31, topic: 'ai_chat.tool_events', group_id: 'tool-audit', user: 'alice', op: 'Insert', payload: { id: 'tool-1', conversation_id: 'c1', tool: 'search', status: 'started' } }),
    makeConsumerMessage({ offset: 32, topic: 'ai_chat.tool_events', group_id: 'tool-audit', user: 'bob', op: 'Update', payload: { id: 'tool-2', conversation_id: 'c2', tool: 'calendar', status: 'completed' } }),
    makeConsumerMessage({ offset: 33, topic: 'ai_chat.tool_events', group_id: 'tool-audit', user: 'carol', op: 'Delete', payload: { id: 'tool-3', conversation_id: 'c3', tool: 'browser', status: 'cancelled', _deleted: true } }),
  ];
  const { client, state } = createConsumerScenarioClient(messages, { parallel: true });
  const contexts = [];
  const audited = [];

  await runConsumer({
    client,
    name: 'tool-audit-worker',
    topic: 'ai_chat.tool_events',
    groupId: 'tool-audit',
    batchSize: 3,
    retry: { maxAttempts: 1, initialBackoffMs: 0, maxBackoffMs: 0 },
    onChange: async (ctx, change) => {
      contexts.push(ctx);
      await new Promise((resolve) => setTimeout(resolve, 34 - change.offset));
      audited.push({
        runKey: ctx.runKey,
        offset: change.offset,
        user: change.user,
        op: change.op,
        tool: change.data.tool,
      });
    },
  });

  assert.equal(new Set(contexts).size, 3);
  assert.deepEqual(sortNumbers(state.ackedOffsets), [31, 32, 33]);
  assert.deepEqual(audited.sort((left, right) => left.offset - right.offset), [
    { runKey: 'tool-audit-worker:ai_chat.tool_events:0:31', offset: 31, user: 'alice', op: 'Insert', tool: 'search' },
    { runKey: 'tool-audit-worker:ai_chat.tool_events:0:32', offset: 32, user: 'bob', op: 'Update', tool: 'calendar' },
    { runKey: 'tool-audit-worker:ai_chat.tool_events:0:33', offset: 33, user: 'carol', op: 'Delete', tool: 'browser' },
  ]);
});