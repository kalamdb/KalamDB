import assert from 'node:assert/strict';
import test from 'node:test';

import { runConsumer } from '../dist/src/index.js';
import { createConsumerScenarioClient, makeConsumerMessage } from './use-case-helpers.mjs';

test('consumer use case 01: chat message insert update and delete maintain a room projection', async () => {
  const messages = [
    makeConsumerMessage({
      offset: 1,
      topic: 'chat.message_cdc',
      group_id: 'chat-projection',
      user: 'alice',
      op: 'Insert',
      payload: { id: 'm1', room_id: 'room-a', author_id: 'alice', body: 'hello team', edited: false, _table: 'chat.messages' },
    }),
    makeConsumerMessage({
      offset: 2,
      topic: 'chat.message_cdc',
      group_id: 'chat-projection',
      user: 'alice',
      op: 'Update',
      payload: { id: 'm1', room_id: 'room-a', author_id: 'alice', body: 'hello team!', edited: true, _table: 'chat.messages' },
    }),
    makeConsumerMessage({
      offset: 3,
      topic: 'chat.message_cdc',
      group_id: 'chat-projection',
      user: 'moderator',
      op: 'Delete',
      payload: { id: 'm1', room_id: 'room-a', author_id: 'alice', _deleted: true, _table: 'chat.messages' },
    }),
  ];
  const { client, state } = createConsumerScenarioClient(messages);
  const roomProjection = new Map();
  const audit = [];

  await runConsumer({
    client,
    name: 'chat-room-projection',
    topic: 'chat.message_cdc',
    groupId: 'chat-projection',
    start: 'earliest',
    retry: { maxAttempts: 1, initialBackoffMs: 0, maxBackoffMs: 0 },
    onChange: async (_ctx, change) => {
      audit.push({ offset: change.offset, op: change.op, user: change.user });
      if (change.op === 'Delete') {
        roomProjection.delete(change.data.id);
        return;
      }
      roomProjection.set(change.data.id, {
        roomId: change.data.room_id,
        body: change.data.body,
        edited: change.data.edited,
      });
    },
  });

  assert.deepEqual(audit, [
    { offset: 1, op: 'Insert', user: 'alice' },
    { offset: 2, op: 'Update', user: 'alice' },
    { offset: 3, op: 'Delete', user: 'moderator' },
  ]);
  assert.deepEqual([...roomProjection.entries()], []);
  assert.deepEqual(state.ackedOffsets, [1, 2, 3]);
  assert.equal(state.consumerOptions[0].topic, 'chat.message_cdc');
});