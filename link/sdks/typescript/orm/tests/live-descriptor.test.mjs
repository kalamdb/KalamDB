import assert from 'node:assert/strict';
import test from 'node:test';
import { asc, desc, eq } from 'drizzle-orm';
import { bigint, text, timestamp } from 'drizzle-orm/pg-core';
import { compileLiveTableDescriptor, kTable, liveTable } from '../dist/index.js';

const messages = kTable.user('chat.messages', {
  id: bigint('id', { mode: 'number' }).primaryKey(),
  room: text('room').notNull(),
  body: text('body').notNull(),
  createdAt: timestamp('created_at', { mode: 'date' }).notNull(),
});

test('compileLiveTableDescriptor builds live-safe SQL and typed projection metadata', () => {
  const descriptor = compileLiveTableDescriptor(messages, {
    where: eq(messages.room, 'main'),
    orderBy: asc(messages.createdAt),
    limit: 20,
  });

  assert.equal(descriptor.mode, 'drizzle');
  assert.equal(descriptor.tableName, 'chat.messages');
  assert.match(descriptor.subscriptionSql, /^SELECT \* FROM chat\.messages WHERE/);
  assert.equal(descriptor.projection.limit, 20);
  assert.deepEqual(descriptor.projection.orderBy, [{ column: 'createdAt', direction: 'asc' }]);
});
import { LiveQueryController } from '@kalamdb/client';

for (const api of ['controller', 'liveTable']) {
  test(`${api} sorts mapped dates before the display limit and retains hidden rows`, async () => {
    let transport;
    let rows;
    const client = {async connect() {}, async live(sql, cb, options) {
      transport = {cb, options};
      return async () => {};
    }};
    const options = {orderBy: desc(messages.createdAt), limit: 1};
    let controller;
    if (api === 'controller') {
      controller = new LiveQueryController(client, compileLiveTableDescriptor(messages, options));
      controller.subscribe(snapshot => {rows = snapshot.rows;});
      await controller.start();
    } else {
      await liveTable(client, messages, value => {rows = value;}, options);
    }
    assert.equal(transport.options.limit, undefined);
    const wire = [
      {id: 1, room: 'r', body: 'b', created_at: '2026-10-19T00:00:00Z'},
      {id: 2, room: 'r', body: 'b', created_at: '2026-10-20T00:00:00Z'},
    ];
    transport.cb(wire.map(transport.options.mapRow));
    assert.deepEqual(rows.map(row => row.id), [2]);
    transport.cb(wire.slice(0, 1).map(transport.options.mapRow));
    assert.deepEqual(rows.map(row => row.id), [1]);
    await controller?.dispose();
  });
}
