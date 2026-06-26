import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdir, mkdtemp, readFile, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { openLocalDb } from '../src/db/local-db.js';
import { syncDbPath } from '../src/lib/paths.js';
import { readLocalSyncState, recordSyncedFile, queuePendingUpload } from '../src/local-cache.js';
import { sha256Hex } from '../src/lib/file-utils.js';

test('pending row prevents treating matching local hash as fully synced', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'okf-pending-skip-'));
  const db = openLocalDb(syncDbPath(dir));
  const path = 'notes.md';
  const hash = sha256Hex(new TextEncoder().encode('content'));
  const now = new Date();

  await recordSyncedFile(db, path, hash, now);
  await queuePendingUpload(db, path, hash, 'offline');

  const state = await readLocalSyncState(db, path);
  const shouldSkipUpload = !state.hasPending && state.cachedHash === hash;
  assert.equal(shouldSkipUpload, false);

  await rm(dir, { recursive: true, force: true });
});

test('maybeSeedSyncFolder does not copy when local files already exist', async () => {
  const parent = await mkdtemp(join(tmpdir(), 'okf-seed-local-'));
  const syncDir = join(parent, 'sync');
  const seedDir = join(parent, 'seed');
  await mkdir(syncDir, { recursive: true });
  await mkdir(seedDir, { recursive: true });
  await writeFile(join(syncDir, 'existing.md'), '# already here\n', 'utf8');
  await writeFile(join(seedDir, 'seed-only.md'), '# seed\n', 'utf8');

  const { maybeSeedSyncFolder } = await import('../src/lib/seed.js');
  try {
    assert.equal(await maybeSeedSyncFolder(syncDir, false, seedDir), 0);
    await assert.rejects(() => readFile(join(syncDir, 'seed-only.md'), 'utf8'));
  } finally {
    await rm(parent, { recursive: true, force: true });
  }
});
