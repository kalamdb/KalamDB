import assert from 'node:assert/strict';
import test from 'node:test';
import { createLiveQueryDescriptor, projectLiveRows } from '../dist/src/index.js';

test('typed descriptors preserve table metadata and projection plan', () => {
  const descriptor = createLiveQueryDescriptor({
    mode: 'drizzle',
    sourceSql: 'SELECT * FROM chat.messages WHERE room = 1',
    tableName: 'chat.messages',
    getKey: ['room_id', 'message_id'],
    projection: {
      orderBy: [{ column: 'created_at', direction: 'asc' }],
      limit: 2,
    },
  });

  const rows = projectLiveRows([
    { message_id: '2', created_at: 2 },
    { message_id: '1', created_at: 1 },
    { message_id: '3', created_at: 3 },
  ], descriptor.projection);

  assert.equal(descriptor.mode, 'drizzle');
  assert.equal(descriptor.tableName, 'chat.messages');
  assert.deepEqual(rows.map((row) => row.message_id), ['1', '2']);
});