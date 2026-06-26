import test from 'node:test';
import assert from 'node:assert/strict';
import { formatSize, InitialSyncTracker, logFilePushed } from '../src/sync-log.js';

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

test('logFilePushed prints path, seq, and size in one line', () => {
  const lines: string[] = [];
  const originalLog = console.log;
  console.log = (line: string) => {
    lines.push(line);
  };

  try {
    logFilePushed({ path: 'index copy 4.md', seq: '12345', sizeBytes: 67 });
  } finally {
    console.log = originalLog;
  }

  assert.deepEqual(lines, ["[sync] pushed path='index copy 4.md' _seq=12345 size=67 B"]);
});
