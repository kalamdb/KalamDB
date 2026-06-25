import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdir, mkdtemp, readFile, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { eq } from 'drizzle-orm';
import { openLocalDb } from '../src/db/local-db.js';
import { syncDbPath } from '../src/lib/paths.js';
import { local_context_files, pending_uploads } from '../src/models/schema.local.js';
import { sha256Hex } from '../src/sync/file-store.js';

test('pending row prevents treating matching local hash as fully synced', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'okf-pending-skip-'));
  const db = openLocalDb(syncDbPath(dir));
  const path = 'notes.md';
  const hash = sha256Hex(new TextEncoder().encode('content'));
  const now = new Date();

  await db.insert(local_context_files).values({
    path,
    sha256: hash,
    created_at: now,
    updated_at: now,
  });

  await db.insert(pending_uploads).values({
    path,
    sha256: hash,
    updated_at: now,
    last_error: 'offline',
  });

  const pending = await db
    .select()
    .from(pending_uploads)
    .where(eq(pending_uploads.path, path));
  const local = await db
    .select()
    .from(local_context_files)
    .where(eq(local_context_files.path, path));

  const shouldSkipUpload = pending.length === 0 && local[0]?.sha256 === hash;
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

  const { maybeSeedSyncFolder } = await import('../src/sync/seed.js');
  try {
    assert.equal(await maybeSeedSyncFolder(syncDir, false, seedDir), 0);
    await assert.rejects(() => readFile(join(syncDir, 'seed-only.md'), 'utf8'));
  } finally {
    await rm(parent, { recursive: true, force: true });
  }
});
