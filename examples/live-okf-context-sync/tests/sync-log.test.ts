import test from 'node:test';
import assert from 'node:assert/strict';
import { formatSize, InitialSyncTracker } from '../src/sync-log.js';

test('formatSize renders human-readable byte sizes', () => {
  assert.equal(formatSize(512), '512 B');
  assert.equal(formatSize(2048), '2.0 KB');
  assert.equal(formatSize(5 * 1024 * 1024), '5.0 MB');
});

test('InitialSyncTracker totals folder events', () => {
  const tracker = new InitialSyncTracker();
  tracker.added = 2;
  tracker.updated = 1;
  tracker.deleted = 1;
  assert.equal(tracker.totalChanges(), 4);
});
