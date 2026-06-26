import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdir, mkdtemp, rename, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { sleep } from '../src/helpers.js';
import { watchSyncFolder } from '../src/folder-watcher.js';

test('watchSyncFolder detects in-place edits', async () => {
  const syncDir = await mkdtemp(join(tmpdir(), 'okf-watch-'));
  const path = 'notes.md';
  const upserts: string[] = [];
  const deletes: string[] = [];

  const watcher = await watchSyncFolder(syncDir, {
    onUpsert: async (relativePath) => {
      upserts.push(relativePath);
    },
    onDelete: async (relativePath) => {
      deletes.push(relativePath);
    },
  });

  try {
    await writeFile(join(syncDir, path), '# v1\n', 'utf8');
    await sleep(600);
    assert.equal(upserts.includes(path), true);

    upserts.length = 0;
    await writeFile(join(syncDir, path), '# v2\n', 'utf8');
    await sleep(600);
    assert.equal(upserts.includes(path), true);
    assert.equal(deletes.includes(path), false);
  } finally {
    await watcher.close();
    await rm(syncDir, { recursive: true, force: true });
  }
});

test('watchSyncFolder suppresses ignored programmatic deletes', async () => {
  const syncDir = await mkdtemp(join(tmpdir(), 'okf-watch-suppress-'));
  const path = 'ignored.md';
  const deletes: string[] = [];

  await writeFile(join(syncDir, path), '# ignore me\n', 'utf8');

  const watcher = await watchSyncFolder(
    syncDir,
    {
      onUpsert: async () => {},
      onDelete: async (relativePath) => {
        deletes.push(relativePath);
      },
    },
    {
      shouldSuppressEvent: (relativePath) => relativePath === path,
    },
  );

  try {
    await rm(join(syncDir, path));
    await sleep(800);
    assert.equal(deletes.includes(path), false);
  } finally {
    await watcher.close();
    await rm(syncDir, { recursive: true, force: true });
  }
});

test('watchSyncFolder treats atomic replace as upsert not delete', async () => {
  const syncDir = await mkdtemp(join(tmpdir(), 'okf-watch-atomic-'));
  const path = 'profile.md';
  const upserts: string[] = [];
  const deletes: string[] = [];

  await writeFile(join(syncDir, path), '# original\n', 'utf8');

  const watcher = await watchSyncFolder(syncDir, {
    onUpsert: async (relativePath) => {
      upserts.push(relativePath);
    },
    onDelete: async (relativePath) => {
      deletes.push(relativePath);
    },
  });

  try {
    upserts.length = 0;

    const tempPath = join(syncDir, `${path}.tmp`);
    await writeFile(tempPath, '# replaced atomically\n', 'utf8');
    await rename(tempPath, join(syncDir, path));
    await sleep(800);

    assert.equal(upserts.includes(path), true, `expected upsert for ${path}, got upserts=${upserts.join(',')}`);
    assert.equal(deletes.includes(path), false, `atomic replace must not delete remote row, deletes=${deletes.join(',')}`);
  } finally {
    await watcher.close();
    await rm(syncDir, { recursive: true, force: true });
  }
});
