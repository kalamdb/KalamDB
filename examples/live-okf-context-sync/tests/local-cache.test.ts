import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { openLocalDb } from '../src/db/local-db.js';
import { syncDbPath } from '../src/lib/paths.js';
import {
  clearPendingUpload,
  hasLocalRecord,
  queuePendingUpload,
  readLocalSyncState,
  recordSyncedFile,
} from '../src/local-cache.js';
import { sha256Hex } from '../src/lib/file-utils.js';

test('readLocalSyncState reports cached hash and pending queue', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'okf-local-cache-'));
  const db = openLocalDb(syncDbPath(dir));
  const path = 'notes.md';
  const hash = sha256Hex(new TextEncoder().encode('content'));

  assert.equal(await hasLocalRecord(db, path), false);

  await recordSyncedFile(db, path, hash);
  assert.equal(await hasLocalRecord(db, path), true);
  assert.deepEqual(await readLocalSyncState(db, path), {
    cachedHash: hash,
    hasPending: false,
  });

  await queuePendingUpload(db, path, hash, 'offline');
  assert.deepEqual(await readLocalSyncState(db, path), {
    cachedHash: hash,
    hasPending: true,
  });

  await clearPendingUpload(db, path);
  assert.deepEqual(await readLocalSyncState(db, path), {
    cachedHash: hash,
    hasPending: false,
  });

  await rm(dir, { recursive: true, force: true });
});
