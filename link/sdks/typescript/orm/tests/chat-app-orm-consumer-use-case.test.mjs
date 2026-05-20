import assert from 'node:assert/strict';
import test from 'node:test';

import { runConsumer } from '@kalamdb/consumer';
import { drizzle } from 'drizzle-orm/pg-proxy';
import { eq } from 'drizzle-orm';
import { boolean, text, timestamp } from 'drizzle-orm/pg-core';
import { executeAsUser, generateSchema, getKalamTableConfig, kalamDriver, kTable } from '../dist/index.js';
import { column, createOrmConsumerClient, createSchemaClient, makeOrmMessage, tableInfo } from './orm-consumer-use-case-helpers.mjs';

test('ORM use case 01: chat app tables generate as shared user stream and runConsumer applies message CDC', async () => {
  const rooms = kTable.shared('chat.rooms', {
    id: text('id').primaryKey(),
    title: text('title').notNull(),
    is_archived: boolean('is_archived').notNull(),
  }, { systemColumns: true });
  const messages = kTable.user('chat.messages', {
    id: text('id').primaryKey(),
    room_id: text('room_id').notNull(),
    author_id: text('author_id').notNull(),
    body: text('body').notNull(),
    edited_at: timestamp('edited_at', { mode: 'string' }),
  }, { systemColumns: true });
  const events = kTable.stream('chat.message_events', {
    id: text('id').primaryKey(),
    message_id: text('message_id').notNull(),
    op: text('op').notNull(),
  }, { systemColumns: true });

  assert.equal(getKalamTableConfig(rooms).tableType, 'shared');
  assert.equal(getKalamTableConfig(messages).tableType, 'user');
  assert.deepEqual(getKalamTableConfig(events).systemColumns, ['_seq']);

  const schema = await generateSchema(createSchemaClient([
    tableInfo('chat', 'rooms', 'Shared', [column('id', 'Text', { ordinal: 1, primary: true, nullable: false }), column('title', 'Text', { ordinal: 2, nullable: false })]),
    tableInfo('chat', 'messages', 'User', [column('id', 'Text', { ordinal: 1, primary: true, nullable: false }), column('body', 'Text', { ordinal: 2, nullable: false })]),
    tableInfo('chat', 'message_events', 'Stream', [column('id', 'Text', { ordinal: 1, primary: true, nullable: false }), column('message_id', 'Text', { ordinal: 2, nullable: false })]),
  ]), { includeSystemColumns: true });
  assert.ok(schema.includes('kTable.shared("chat.rooms"'));
  assert.ok(schema.includes('kTable.user("chat.messages"'));
  assert.ok(schema.includes('kTable.stream("chat.message_events"'));

  const topicName = 'chat.message_events_topic';
  const { client, state } = createOrmConsumerClient([
    makeOrmMessage({ offset: 1, topic: topicName, group_id: 'chat-orm-worker', user: 'alice', op: 'Insert', payload: { id: 'm1', room_id: 'r1', author_id: 'alice', body: 'hello' } }),
    makeOrmMessage({ offset: 2, topic: topicName, group_id: 'chat-orm-worker', user: 'alice', op: 'Update', payload: { id: 'm1', room_id: 'r1', author_id: 'alice', body: 'hello edited', edited_at: '2026-05-20T10:00:00' } }),
    makeOrmMessage({ offset: 3, topic: topicName, group_id: 'chat-orm-worker', user: 'moderator', op: 'Delete', payload: { id: 'm1' } }),
  ]);
  const db = drizzle(kalamDriver(client));

  await runConsumer({
    client,
    name: 'chat-orm-worker',
    topic: topicName,
    groupId: 'chat-orm-worker',
    start: 'earliest',
    retry: { maxAttempts: 1, initialBackoffMs: 0, maxBackoffMs: 0 },
    onChange: async (_ctx, change) => {
      if (change.op === 'Insert') {
        await executeAsUser(client, db.insert(messages).values(change.data), change.user);
      } else if (change.op === 'Update') {
        await executeAsUser(client, db.update(messages).set({ body: String(change.data.body), edited_at: String(change.data.edited_at) }).where(eq(messages.id, String(change.data.id))), change.user);
      } else if (change.op === 'Delete') {
        await executeAsUser(client, db.delete(messages).where(eq(messages.id, String(change.data.id))), change.user);
      }
    },
  });

  assert.equal(state.consumerOptions[0].topic, topicName);
  assert.deepEqual(state.ackedOffsets, [1, 2, 3]);
  assert.match(state.executeAsUserCalls[0].sql, /insert into chat\.messages/i);
  assert.match(state.executeAsUserCalls[1].sql, /update chat\.messages set/i);
  assert.match(state.executeAsUserCalls[2].sql, /delete from chat\.messages/i);
  assert.deepEqual(state.executeAsUserCalls.map((call) => call.user), ['alice', 'alice', 'moderator']);
});