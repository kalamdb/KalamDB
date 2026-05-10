import assert from 'node:assert/strict';
import test from 'node:test';
import { asc, eq } from 'drizzle-orm';
import { bigint, text, timestamp } from 'drizzle-orm/pg-core';
import { compileLiveTableDescriptor, kTable } from '../dist/index.js';

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
  assert.deepEqual(descriptor.projection.orderBy, [{ column: 'created_at', direction: 'asc' }]);
});