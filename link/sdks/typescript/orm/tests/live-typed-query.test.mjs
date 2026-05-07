import assert from 'node:assert/strict';
import test from 'node:test';
import { bigint, text } from 'drizzle-orm/pg-core';
import { compileLiveTableDescriptor, kTable } from '../dist/index.js';

const threads = kTable.user('chat.threads', {
  id: bigint('id', { mode: 'number' }).primaryKey(),
  title: text('title').notNull(),
});

test('compileLiveTableDescriptor maps RowData into Drizzle-shaped rows', () => {
  const descriptor = compileLiveTableDescriptor(threads);
  const row = descriptor.mapRow({
    id: { toJson: () => 42 },
    title: { toJson: () => 'Support' },
  });

  assert.deepEqual(row, { id: 42, title: 'Support' });
});