import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdir, mkdtemp, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import {
  indexDir,
  isExcludedWatchPath,
  isSafeSyncPath,
  listSyncFiles,
  shouldIgnoreWatchAbsolutePath,
  syncDbPath,
} from '../src/lib/paths.js';

test('syncDbPath places sqlite under .index/', () => {
  assert.equal(syncDbPath('/data'), '/data/.index/sync.db');
  assert.equal(indexDir('/data'), '/data/.index');
});

test('isSafeSyncPath accepts normal relative paths', () => {
  assert.equal(isSafeSyncPath('index.md'), true);
  assert.equal(isSafeSyncPath('notes/getting-started.md'), true);
  assert.equal(isSafeSyncPath('deep/nested/file.txt'), true);
});

test('isSafeSyncPath rejects unsafe paths', () => {
  assert.equal(isSafeSyncPath(''), false);
  assert.equal(isSafeSyncPath('.index/sync.db'), false);
  assert.equal(isSafeSyncPath('.index/foo'), false);
  assert.equal(isSafeSyncPath('.git/config'), false);
  assert.equal(isSafeSyncPath('.DS_Store'), false);
  assert.equal(isSafeSyncPath('tmp/92ff226c-alice/upload'), false);
  assert.equal(isSafeSyncPath('tmp'), false);
  assert.equal(isSafeSyncPath('../escape.md'), false);
  assert.equal(isSafeSyncPath('notes/../../etc/passwd'), false);
  assert.equal(isSafeSyncPath('/absolute.md'), false);
  assert.equal(isSafeSyncPath('C:/windows/system.ini'), false);
});

test('isExcludedWatchPath ignores local index, git, and sqlite artifacts', () => {
  assert.equal(isExcludedWatchPath('.index/sync.db'), true);
  assert.equal(isExcludedWatchPath('.index/sync.db-wal'), true);
  assert.equal(isExcludedWatchPath('.index/sync.db-shm'), true);
  assert.equal(isExcludedWatchPath('.git/config'), true);
  assert.equal(isExcludedWatchPath('.DS_Store'), true);
  assert.equal(isExcludedWatchPath('notes/readme.md'), false);
});

test('shouldIgnoreWatchAbsolutePath never ignores the watch root directory', () => {
  const syncDir = '/project/data';
  assert.equal(shouldIgnoreWatchAbsolutePath(syncDir, syncDir), false);
  assert.equal(shouldIgnoreWatchAbsolutePath(syncDir, `${syncDir}/notes.md`), false);
  assert.equal(shouldIgnoreWatchAbsolutePath(syncDir, `${syncDir}/.index/sync.db`), true);
});

test('listSyncFiles skips .index and .git but includes user files', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'okf-list-'));
  try {
    await mkdir(join(dir, 'notes'), { recursive: true });
    await mkdir(join(dir, '.git', 'objects'), { recursive: true });
    await mkdir(indexDir(dir), { recursive: true });
    await writeFile(join(dir, 'index.md'), '# hi', 'utf8');
    await writeFile(join(dir, 'notes', 'a.md'), '# a', 'utf8');
    await writeFile(join(dir, '.DS_Store'), 'mac', 'utf8');
    await writeFile(join(indexDir(dir), 'sync.db'), 'sqlite', 'utf8');
    await writeFile(join(dir, '.git', 'config'), 'git', 'utf8');

    const files = await listSyncFiles(dir);
    assert.deepEqual(files.sort(), ['index.md', 'notes/a.md']);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});
