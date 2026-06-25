import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { eq } from 'drizzle-orm';
import { openLocalDb } from '../src/db/local-db.js';
import { indexDir, syncDbPath } from '../src/lib/paths.js';
import { maybeSeedSyncFolder } from '../src/sync/seed.js';
import { local_context_files, pending_uploads } from '../src/models/schema.local.js';
import { sha256Hex } from '../src/sync/file-store.js';

test('maybeSeedSyncFolder copies seed/ when local and server are empty', async () => {
  const parent = await mkdtemp(join(tmpdir(), 'okf-seed-'));
  const syncDir = join(parent, 'sync');
  const seedDir = join(parent, 'seed');
  await mkdir(join(seedDir, 'notes'), { recursive: true });
  await writeFile(join(seedDir, 'index.md'), '# hello seed\n', 'utf8');

  try {
    const created = await maybeSeedSyncFolder(syncDir, false, seedDir);
    assert.equal(created, 1);
    assert.equal(await maybeSeedSyncFolder(syncDir, false, seedDir), 0);

    const index = await readFile(join(syncDir, 'index.md'), 'utf8');
    assert.match(index, /hello seed/);
  } finally {
    await rm(parent, { recursive: true, force: true });
  }
});

test('maybeSeedSyncFolder skips when server already has files', async () => {
  const parent = await mkdtemp(join(tmpdir(), 'okf-seed-skip-'));
  const syncDir = join(parent, 'sync');
  const seedDir = join(parent, 'seed');
  await mkdir(seedDir, { recursive: true });
  await writeFile(join(seedDir, 'index.md'), '# seed\n', 'utf8');

  try {
    assert.equal(await maybeSeedSyncFolder(syncDir, true, seedDir), 0);
    await assert.rejects(() => readFile(join(syncDir, 'index.md'), 'utf8'));
  } finally {
    await rm(parent, { recursive: true, force: true });
  }
});

test('local sqlite lives under .index/sync.db', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'okf-sqlite-'));
  const dbPath = syncDbPath(dir);
  assert.equal(dbPath, join(dir, '.index', 'sync.db'));

  const db = openLocalDb(dbPath);
  const now = new Date('2026-06-25T12:00:00.000Z');
  const later = new Date('2026-06-25T13:00:00.000Z');
  const hash = sha256Hex(new TextEncoder().encode('hello sync'));

  await db.insert(local_context_files).values({
    path: 'notes.md',
    sha256: hash,
    created_at: now,
    updated_at: now,
  });

  await db.insert(local_context_files).values({
    path: 'notes.md',
    sha256: hash,
    created_at: now,
    updated_at: now,
  }).onConflictDoUpdate({
    target: local_context_files.path,
    set: { sha256: 'updated', updated_at: later },
  });

  const rows = await db
    .select()
    .from(local_context_files)
    .where(eq(local_context_files.path, 'notes.md'));
  assert.equal(rows[0]?.sha256, 'updated');

  await db.insert(pending_uploads).values({
    path: 'notes.md',
    sha256: hash,
    updated_at: now,
    last_error: 'offline',
  });

  await db.insert(pending_uploads).values({
    path: 'notes.md',
    sha256: hash,
    updated_at: now,
    last_error: 'offline',
  }).onConflictDoUpdate({
    target: pending_uploads.path,
    set: { sha256: hash, updated_at: later, last_error: 'retry' },
  });

  const pending = await db
    .select()
    .from(pending_uploads)
    .where(eq(pending_uploads.path, 'notes.md'));
  assert.equal(pending[0]?.last_error, 'retry');

  await rm(indexDir(dir), { recursive: true, force: true });
  await rm(dir, { recursive: true, force: true });
});
