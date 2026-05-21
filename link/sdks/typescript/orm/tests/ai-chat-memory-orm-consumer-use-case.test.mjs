import assert from 'node:assert/strict';
import test from 'node:test';

import { runConsumer } from '@kalamdb/consumer';
import { drizzle } from 'drizzle-orm/pg-proxy';
import { jsonb, text, timestamp } from 'drizzle-orm/pg-core';
import { embedding, executeAsUser, file, generateSchema, kalamDriver, kTable } from '../dist/index.js';
import { column, createOrmConsumerClient, createSchemaClient, makeOrmMessage, tableInfo } from './orm-consumer-use-case-helpers.mjs';

test('ORM use case 02: AI chat memory tables generate complex columns and consumer stores embeddings', async () => {
  const conversations = kTable.shared('ai.conversations', {
    id: text('id').primaryKey(),
    title: text('title').notNull(),
    metadata: jsonb('metadata'),
  });
  const memories = kTable.user('ai.memories', {
    id: text('id').primaryKey(),
    conversation_id: text('conversation_id').notNull(),
    user_id: text('user_id').notNull(),
    snippet: text('snippet').notNull(),
    vector: embedding('vector', 8),
    source_file: file('source_file'),
    created_at: timestamp('created_at', { mode: 'string' }),
  });
  const turnEvents = kTable.stream('ai.turn_events', {
    id: text('id').primaryKey(),
    conversation_id: text('conversation_id').notNull(),
    payload: jsonb('payload'),
  });

  const schema = await generateSchema(createSchemaClient([
    tableInfo('ai', 'conversations', 'Shared', [column('id', 'Text', { ordinal: 1, primary: true, nullable: false }), column('metadata', 'JSONB', { ordinal: 2 })]),
    tableInfo('ai', 'memories', 'User', [column('id', 'Text', { ordinal: 1, primary: true, nullable: false }), column('vector', 'Embedding(8)', { ordinal: 2 }), column('source_file', 'File', { ordinal: 3 })]),
    tableInfo('ai', 'turn_events', 'Stream', [column('id', 'Text', { ordinal: 1, primary: true, nullable: false }), column('payload', 'Json', { ordinal: 2 })]),
  ]));
  assert.ok(schema.includes('embedding("vector", 8)'));
  assert.ok(schema.includes('file("source_file")'));
  assert.ok(schema.includes('jsonb("payload")'));
  assert.ok(schema.includes('kTable.stream("ai.turn_events"'));

  const topicName = 'ai.turn_events_topic';
  const { client, state } = createOrmConsumerClient([
    makeOrmMessage({ offset: 10, topic: topicName, group_id: 'ai-memory-worker', user: 'user-42', op: 'Insert', payload: { id: 'turn-10', conversation_id: 'conv-42', body: 'Remember that the customer prefers email updates.' } }),
  ]);
  const db = drizzle(kalamDriver(client));

  await runConsumer({
    client,
    name: 'ai-memory-worker',
    topic: topicName,
    groupId: 'ai-memory-worker',
    retry: { maxAttempts: 1, initialBackoffMs: 0, maxBackoffMs: 0 },
    onChange: async (_ctx, change) => {
      await executeAsUser(client, db.insert(memories).values({
        id: `memory-${change.data.id}`,
        conversation_id: String(change.data.conversation_id),
        user_id: String(change.user),
        snippet: String(change.data.body),
        vector: [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8],
        source_file: null,
        created_at: '2026-05-20T12:00:00',
      }), change.user);
    },
  });

  assert.equal(conversations.id.name, 'id');
  assert.equal(turnEvents.id.name, 'id');
  assert.deepEqual(state.ackedOffsets, [10]);
  assert.match(state.executeAsUserCalls[0].sql, /insert into ai\.memories/i);
  assert.equal(state.executeAsUserCalls[0].user, 'user-42');
});