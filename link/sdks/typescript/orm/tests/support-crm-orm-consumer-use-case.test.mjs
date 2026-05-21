import assert from 'node:assert/strict';
import test from 'node:test';

import { runConsumer } from '@kalamdb/consumer';
import { drizzle } from 'drizzle-orm/pg-proxy';
import { eq } from 'drizzle-orm';
import { integer, jsonb, text, timestamp } from 'drizzle-orm/pg-core';
import { executeAsUser, generateSchema, kalamDriver, kTable } from '../dist/index.js';
import { column, createOrmConsumerClient, createSchemaClient, makeOrmMessage, tableInfo } from './orm-consumer-use-case-helpers.mjs';

test('ORM use case 04: support CRM tables generate and consumer writes private notes plus ticket priority changes', async () => {
  const tickets = kTable.shared('support.tickets', {
    id: text('id').primaryKey(),
    account_id: text('account_id').notNull(),
    priority: integer('priority').notNull(),
    status: text('status').notNull(),
    metadata: jsonb('metadata'),
  });
  const notes = kTable.user('support.private_notes', {
    id: text('id').primaryKey(),
    ticket_id: text('ticket_id').notNull(),
    author_id: text('author_id').notNull(),
    body: text('body').notNull(),
    created_at: timestamp('created_at', { mode: 'string' }),
  });
  const events = kTable.stream('support.ticket_events', {
    id: text('id').primaryKey(),
    ticket_id: text('ticket_id').notNull(),
    event_type: text('event_type').notNull(),
  });

  const schema = await generateSchema(createSchemaClient([
    tableInfo('support', 'tickets', 'Shared', [column('id', 'Text', { ordinal: 1, primary: true, nullable: false }), column('metadata', 'Json', { ordinal: 2 })]),
    tableInfo('support', 'private_notes', 'User', [column('id', 'Text', { ordinal: 1, primary: true, nullable: false }), column('created_at', 'DateTime', { ordinal: 2 })]),
    tableInfo('support', 'ticket_events', 'Stream', [column('id', 'Text', { ordinal: 1, primary: true, nullable: false }), column('event_type', 'Text', { ordinal: 2, nullable: false })]),
  ]));
  assert.ok(schema.includes('jsonb("metadata")'));
  assert.ok(schema.includes('kTable.user("support.private_notes"'));

  const topicName = 'support.ticket_events_topic';
  const { client, state } = createOrmConsumerClient([
    makeOrmMessage({ offset: 30, topic: topicName, group_id: 'support-crm-worker', user: 'agent-1', op: 'Update', payload: { id: 'ticket-30', priority: 9, note: 'Escalated because SLA is under 15 minutes.' } }),
  ]);
  const db = drizzle(kalamDriver(client));

  await runConsumer({
    client,
    name: 'support-crm-worker',
    topic: topicName,
    groupId: 'support-crm-worker',
    retry: { maxAttempts: 1, initialBackoffMs: 0, maxBackoffMs: 0 },
    onChange: async (_ctx, change) => {
      await executeAsUser(client, db.update(tickets).set({ priority: Number(change.data.priority), status: 'escalated' }).where(eq(tickets.id, String(change.data.id))), change.user);
      await executeAsUser(client, db.insert(notes).values({ id: `note-${change.data.id}`, ticket_id: String(change.data.id), author_id: String(change.user), body: String(change.data.note), created_at: '2026-05-20T12:30:00' }), change.user);
    },
  });

  assert.equal(events.id.name, 'id');
  assert.match(state.executeAsUserCalls[0].sql, /update support\.tickets set/i);
  assert.match(state.executeAsUserCalls[1].sql, /insert into support\.private_notes/i);
  assert.deepEqual(state.executeAsUserCalls.map((call) => call.user), ['agent-1', 'agent-1']);
  assert.deepEqual(state.ackedOffsets, [30]);
});