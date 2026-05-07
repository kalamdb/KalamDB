import assert from 'node:assert/strict';
import test from 'node:test';
import { LiveQueryDescriptorError, createRawSqlLiveDescriptor, normalizeLiveSql } from '../dist/src/index.js';

test('normalizeLiveSql strips ORDER BY and LIMIT into projection', () => {
  const normalized = normalizeLiveSql(
    'SELECT * FROM chat.messages WHERE room = 1 ORDER BY created_at DESC LIMIT 5',
  );

  assert.equal(normalized.subscriptionSql, 'SELECT * FROM chat.messages WHERE room = 1');
  assert.equal(normalized.tableName, 'chat.messages');
  assert.deepEqual(normalized.projection.orderBy, [{ column: 'created_at', direction: 'desc' }]);
  assert.equal(normalized.projection.limit, 5);
});

test('createRawSqlLiveDescriptor rejects unsupported raw SQL shapes', () => {
  assert.throws(
    () => createRawSqlLiveDescriptor('SELECT room, count(*) FROM chat.messages GROUP BY room'),
    LiveQueryDescriptorError,
  );
});