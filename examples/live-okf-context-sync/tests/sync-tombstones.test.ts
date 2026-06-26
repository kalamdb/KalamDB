import test from 'node:test';
import assert from 'node:assert/strict';
import { SyncTombstones, TOMBSTONE_ANY } from '../src/lib/sync-tombstones.js';

test('SyncTombstones blocks pull and push for the same deleted hash', () => {
  const tombstones = new SyncTombstones();
  tombstones.mark('notes.md', 'abc123');

  assert.equal(tombstones.shouldBlockRemotePull('notes.md', 'abc123'), true);
  assert.equal(tombstones.shouldBlockLocalPush('notes.md', 'abc123'), true);
});

test('SyncTombstones allows new content with a different hash', () => {
  const tombstones = new SyncTombstones();
  tombstones.mark('notes.md', 'old-hash');

  assert.equal(tombstones.shouldBlockLocalPush('notes.md', 'new-hash'), false);
  assert.equal(tombstones.shouldBlockRemotePull('notes.md', 'new-hash'), false);
});

test('SyncTombstones without hash blocks every remote pull', () => {
  const tombstones = new SyncTombstones();
  tombstones.mark('notes.md', null);

  assert.equal(tombstones.shouldBlockRemotePull('notes.md', 'any-hash'), true);
  assert.equal(tombstones.shouldBlockLocalPush('notes.md', 'any-hash'), true);
});
