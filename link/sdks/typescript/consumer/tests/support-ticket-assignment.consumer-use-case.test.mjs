import assert from 'node:assert/strict';
import test from 'node:test';

import { runConsumer } from '../dist/src/index.js';
import { createConsumerScenarioClient, makeConsumerMessage } from './use-case-helpers.mjs';

test('consumer use case 10: support ticket assignment keeps deprecated onMessage integrations working', async () => {
  const message = makeConsumerMessage({
    offset: 81,
    topic: 'support.ticket_events',
    group_id: 'ticket-assignment-worker',
    user: 'support-bot',
    op: 'Insert',
    payload: { id: 'ticket-81', account_id: 'acct-1', priority: 'urgent', channel: 'chat' },
  });
  const { client, state } = createConsumerScenarioClient([message], {
    queryAll: async () => [{ user_id: 'agent-1', open_count: 2 }, { user_id: 'agent-2', open_count: 4 }],
  });
  const assigned = [];

  await runConsumer({
    client,
    name: 'ticket-assignment-worker',
    topic: 'support.ticket_events',
    groupId: 'ticket-assignment-worker',
    retry: { maxAttempts: 1, initialBackoffMs: 0, maxBackoffMs: 0 },
    onMessage: async (ctx, change) => {
      const agents = await ctx.queryAll('SELECT user_id, open_count FROM support.agent_load ORDER BY open_count ASC', []);
      const assignee = agents[0].user_id;
      assigned.push({ ticketId: change.data.id, assignee, hasMessageOnCtx: 'message' in ctx });
      await ctx.sql('UPDATE support.tickets SET assignee_id = $2 WHERE id = $1', [change.data.id, assignee]);
    },
  });

  assert.deepEqual(assigned, [{ ticketId: 'ticket-81', assignee: 'agent-1', hasMessageOnCtx: false }]);
  assert.deepEqual(state.queryAllCalls.map((call) => call.sql), ['SELECT user_id, open_count FROM support.agent_load ORDER BY open_count ASC']);
  assert.deepEqual(state.queryCalls[0].params, ['ticket-81', 'agent-1']);
  assert.deepEqual(state.ackedOffsets, [81]);
});