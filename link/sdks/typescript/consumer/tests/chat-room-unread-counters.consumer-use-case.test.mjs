import assert from 'node:assert/strict';
import test from 'node:test';

import { runConsumer } from '../dist/src/index.js';
import { createConsumerScenarioClient, makeConsumerMessage } from './use-case-helpers.mjs';

test('consumer use case 02: chat fanout updates unread counters for each recipient', async () => {
  const messages = [
    makeConsumerMessage({
      offset: 10,
      topic: 'chat.room_events',
      group_id: 'unread-counter-worker',
      user: 'alice',
      op: 'Insert',
      payload: {
        row: {
          id: 'm10',
          room_id: 'room-red',
          sender_id: 'alice',
          recipient_ids: ['alice', 'bob', 'carol'],
          body: 'standup moved to 10',
        },
      },
    }),
    makeConsumerMessage({
      offset: 11,
      topic: 'chat.room_events',
      group_id: 'unread-counter-worker',
      user: 'bob',
      op: 'Insert',
      payload: {
        row: {
          id: 'm11',
          room_id: 'room-red',
          sender_id: 'bob',
          recipient_ids: ['alice', 'bob', 'carol'],
          body: 'got it',
        },
      },
    }),
  ];
  const { client, state } = createConsumerScenarioClient(messages);

  await runConsumer({
    client,
    name: 'unread-counter-worker',
    topic: 'chat.room_events',
    groupId: 'unread-counter-worker',
    start: 'earliest',
    retry: { maxAttempts: 1, initialBackoffMs: 0, maxBackoffMs: 0 },
    onChange: async (ctx, change) => {
      const recipients = change.data.recipient_ids.filter((recipientId) => recipientId !== change.data.sender_id);
      await Promise.all(recipients.map((recipientId) => ctx.sql(
        'UPDATE chat.room_members SET unread_count = unread_count + 1 WHERE room_id = $1 AND user_id = $2',
        [change.data.room_id, recipientId],
      )));
    },
  });

  assert.deepEqual(state.queryCalls.map((call) => call.params), [
    ['room-red', 'bob'],
    ['room-red', 'carol'],
    ['room-red', 'alice'],
    ['room-red', 'carol'],
  ]);
  assert.deepEqual(state.ackedOffsets, [10, 11]);
});