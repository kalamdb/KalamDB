import assert from 'node:assert/strict';
import test from 'node:test';

import { runConsumer } from '@kalamdb/consumer';
import { drizzle } from 'drizzle-orm/pg-proxy';
import { eq } from 'drizzle-orm';
import { jsonb, numeric, text, timestamp } from 'drizzle-orm/pg-core';
import { executeAsUser, generateSchema, kalamDriver, kTable } from '../dist/index.js';
import { column, createOrmConsumerClient, createSchemaClient, makeOrmMessage, tableInfo } from './orm-consumer-use-case-helpers.mjs';

test('ORM use case 05: commerce fulfillment tables generate and consumer dedupes order outbox rows', async () => {
  const orders = kTable.shared('commerce.orders', {
    id: text('id').primaryKey(),
    account_id: text('account_id').notNull(),
    status: text('status').notNull(),
    total: numeric('total', { precision: 12, scale: 2 }).notNull(),
    metadata: jsonb('metadata'),
  });
  const carts = kTable.user('commerce.carts', {
    id: text('id').primaryKey(),
    account_id: text('account_id').notNull(),
    payload: jsonb('payload'),
  });
  const orderEvents = kTable.stream('commerce.order_events', {
    id: text('id').primaryKey(),
    order_id: text('order_id').notNull(),
    event_type: text('event_type').notNull(),
    created_at: timestamp('created_at', { mode: 'string' }),
  });
  const outbox = kTable.user('commerce.fulfillment_outbox', {
    id: text('id').primaryKey(),
    order_id: text('order_id').notNull(),
    event_type: text('event_type').notNull(),
  });

  const schema = await generateSchema(createSchemaClient([
    tableInfo('commerce', 'orders', 'Shared', [column('id', 'Text', { ordinal: 1, primary: true, nullable: false }), column('total', 'Decimal(12, 2)', { ordinal: 2, nullable: false })]),
    tableInfo('commerce', 'carts', 'User', [column('id', 'Text', { ordinal: 1, primary: true, nullable: false }), column('payload', 'Json', { ordinal: 2 })]),
    tableInfo('commerce', 'order_events', 'Stream', [column('id', 'Text', { ordinal: 1, primary: true, nullable: false }), column('created_at', 'Timestamp', { ordinal: 2 })]),
    tableInfo('commerce', 'fulfillment_outbox', 'User', [column('id', 'Text', { ordinal: 1, primary: true, nullable: false }), column('event_type', 'Text', { ordinal: 2, nullable: false })]),
  ]));
  assert.ok(schema.includes('numeric("total", { precision: 12, scale: 2 }).notNull()'));
  assert.ok(schema.includes('kTable.stream("commerce.order_events"'));
  assert.ok(schema.includes('kTable.user("commerce.carts"'));

  const topicName = 'commerce.order_events_topic';
  const dispatched = new Set();
  const { client, state } = createOrmConsumerClient([
    makeOrmMessage({ offset: 40, topic: topicName, group_id: 'fulfillment-worker', user: 'checkout-service', op: 'Insert', key: 'order-40:paid', payload: { id: 'evt-40', order_id: 'order-40', account_id: 'acct-40', event_type: 'paid' } }),
    makeOrmMessage({ offset: 41, topic: topicName, group_id: 'fulfillment-worker', user: 'checkout-service', op: 'Insert', key: 'order-40:paid', payload: { id: 'evt-40-duplicate', order_id: 'order-40', account_id: 'acct-40', event_type: 'paid' } }),
  ], {
    queryOne: async (_sql, params) => dispatched.has(params[0]) ? { id: params[0] } : null,
  });
  const db = drizzle(kalamDriver(client));

  await runConsumer({
    client,
    name: 'fulfillment-worker',
    topic: topicName,
    groupId: 'fulfillment-worker',
    runKeyFactory: ({ message }) => `fulfillment:${message.key}`,
    retry: { maxAttempts: 1, initialBackoffMs: 0, maxBackoffMs: 0 },
    onChange: async (ctx, change) => {
      const duplicate = await ctx.queryOne('SELECT id FROM commerce.fulfillment_outbox WHERE id = $1', [ctx.runKey]);
      if (duplicate) return;
      dispatched.add(ctx.runKey);
      await executeAsUser(client, db.update(orders).set({ status: String(change.data.event_type) }).where(eq(orders.id, String(change.data.order_id))), 'system');
      await executeAsUser(client, db.insert(outbox).values({ id: ctx.runKey, order_id: String(change.data.order_id), event_type: String(change.data.event_type) }), 'system');
    },
  });

  assert.equal(carts.id.name, 'id');
  assert.equal(orderEvents.id.name, 'id');
  assert.deepEqual(state.queryOneCalls.map((call) => call.params[0]), ['fulfillment:order-40:paid', 'fulfillment:order-40:paid']);
  assert.equal(state.executeAsUserCalls.length, 2);
  assert.match(state.executeAsUserCalls[0].sql, /update commerce\.orders set/i);
  assert.match(state.executeAsUserCalls[1].sql, /insert into commerce\.fulfillment_outbox/i);
  assert.deepEqual(state.ackedOffsets, [40, 41]);
});