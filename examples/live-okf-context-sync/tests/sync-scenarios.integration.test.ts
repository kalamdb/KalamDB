import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { eq } from 'drizzle-orm';
import { createDb } from '../src/db/client.js';
import { fetchRemoteHash, sha256Hex } from '../src/sync/file-store.js';
import { openLocalDb } from '../src/db/local-db.js';
import { syncDbPath } from '../src/lib/paths.js';
import { pending_uploads } from '../src/models/schema.local.js';
import { FolderSyncApp } from '../src/sync/sync-app.js';
import {
  aliceConnection,
  deletePaths,
  login,
  RUN_INTEGRATION,
  serverHealthy,
  stopSyncApp,
  uniquePath,
} from './sync.helpers.js';

const integration = { skip: !RUN_INTEGRATION };

test('integration: update overwrites remote file content', integration, async () => {
  if (!(await serverHealthy())) {
    return;
  }

  const syncDir = await mkdtemp(join(tmpdir(), 'okf-update-'));
  const path = uniquePath('update') + '.md';
  const v1 = '# version one\n';
  const v2 = '# version two\n';
  const paths = [path];
  let app: FolderSyncApp | undefined;

  try {
    app = new FolderSyncApp({ syncDir, connection: aliceConnection(), watch: false });
    await app.start();

    await writeFile(join(syncDir, path), v1, 'utf8');
    await app.pushLocalFile(path);

    const db = createDb((await login('alice', 'alice123')).client);
    assert.equal(await fetchRemoteHash(db, path), sha256Hex(new TextEncoder().encode(v1)));

    await writeFile(join(syncDir, path), v2, 'utf8');
    await app.pushLocalFile(path);
    assert.equal(await fetchRemoteHash(db, path), sha256Hex(new TextEncoder().encode(v2)));
  } finally {
    await stopSyncApp(app);
    await deletePaths(paths).catch(() => undefined);
    await rm(syncDir, { recursive: true, force: true });
  }
});

test('integration: second client receives files pushed by the first', integration, async () => {
  if (!(await serverHealthy())) {
    return;
  }

  const syncDirA = await mkdtemp(join(tmpdir(), 'okf-client-a-'));
  const syncDirB = await mkdtemp(join(tmpdir(), 'okf-client-b-'));
  const path = uniquePath('shared') + '/note.md';
  const content = `# shared ${Date.now()}\n`;
  const paths = [path];
  let clientA: FolderSyncApp | undefined;
  let clientB: FolderSyncApp | undefined;

  try {
    clientA = new FolderSyncApp({ syncDir: syncDirA, connection: aliceConnection(), watch: false });
    await clientA.start();
    await mkdir(join(syncDirA, path.split('/').slice(0, -1).join('/')), { recursive: true });
    await writeFile(join(syncDirA, path), content, 'utf8');
    await clientA.pushLocalFile(path);
    await stopSyncApp(clientA);
    clientA = undefined;

    clientB = new FolderSyncApp({ syncDir: syncDirB, connection: aliceConnection(), watch: false });
    await clientB.start();
    await clientB.waitForLocalFiles([path], 20_000);
    assert.equal(await clientB.readLocalFile(path), content);
  } finally {
    await stopSyncApp(clientA);
    await stopSyncApp(clientB);
    await deletePaths(paths).catch(() => undefined);
    await rm(syncDirA, { recursive: true, force: true });
    await rm(syncDirB, { recursive: true, force: true });
  }
});

test('integration: pending upload queue retries after simulated failure', integration, async () => {
  if (!(await serverHealthy())) {
    return;
  }

  const syncDir = await mkdtemp(join(tmpdir(), 'okf-pending-'));
  const path = uniquePath('pending') + '.md';
  const content = `# pending retry ${Date.now()}\n`;
  const paths = [path];
  let app: FolderSyncApp | undefined;

  try {
    app = new FolderSyncApp({ syncDir, connection: aliceConnection(), watch: false });
    await app.start();

    await writeFile(join(syncDir, path), content, 'utf8');

    const localDb = openLocalDb(syncDbPath(syncDir));
    await localDb.insert(pending_uploads).values({
      path,
      sha256: sha256Hex(new TextEncoder().encode(content)),
      updated_at: new Date(),
      last_error: 'simulated offline',
    });

    await app.flushPendingUploads();

    const { client } = await login('alice', 'alice123');
    const db = createDb(client);
    assert.equal(await fetchRemoteHash(db, path), sha256Hex(new TextEncoder().encode(content)));
    await client.disconnect();

    const queued = await localDb
      .select()
      .from(pending_uploads)
      .where(eq(pending_uploads.path, path));
    assert.equal(queued.length, 0);
  } finally {
    await stopSyncApp(app);
    await deletePaths(paths).catch(() => undefined);
    await rm(syncDir, { recursive: true, force: true });
  }
});

test('integration: pushLocalFile skips unchanged files', integration, async () => {
  if (!(await serverHealthy())) {
    return;
  }

  const syncDir = await mkdtemp(join(tmpdir(), 'okf-skip-'));
  const path = uniquePath('skip') + '.md';
  const content = '# unchanged\n';
  const paths = [path];
  let app: FolderSyncApp | undefined;

  try {
    app = new FolderSyncApp({ syncDir, connection: aliceConnection(), watch: false });
    await app.start();

    await writeFile(join(syncDir, path), content, 'utf8');
    await app.pushLocalFile(path);

    const { client } = await login('alice', 'alice123');
    const hashBefore = await fetchRemoteHash(createDb(client), path);

    await app.pushLocalFile(path);
    const hashAfter = await fetchRemoteHash(createDb(client), path);
    assert.equal(hashBefore, hashAfter);
    await client.disconnect();
  } finally {
    await stopSyncApp(app);
    await deletePaths(paths).catch(() => undefined);
    await rm(syncDir, { recursive: true, force: true });
  }
});

test('integration: remote delete removes local copy', integration, async () => {
  if (!(await serverHealthy())) {
    return;
  }

  const syncDir = await mkdtemp(join(tmpdir(), 'okf-delete-'));
  const path = uniquePath('delete') + '.md';
  const content = '# delete me\n';
  const paths = [path];
  let app: FolderSyncApp | undefined;

  try {
    app = new FolderSyncApp({ syncDir, connection: aliceConnection(), watch: false });
    await app.start();

    await writeFile(join(syncDir, path), content, 'utf8');
    await app.pushLocalFile(path);
    await app.waitForLocalFiles([path]);

    await deletePaths([path]);
    await app.waitForLocalFileAbsent(path, 20_000);
  } finally {
    await stopSyncApp(app);
    await deletePaths(paths).catch(() => undefined);
    await rm(syncDir, { recursive: true, force: true });
  }
});

test('integration: .index is never included in uploads', integration, async () => {
  if (!(await serverHealthy())) {
    return;
  }

  const syncDir = await mkdtemp(join(tmpdir(), 'okf-index-'));
  const userPath = uniquePath('user') + '.md';
  const paths = [userPath];
  let app: FolderSyncApp | undefined;

  try {
    app = new FolderSyncApp({ syncDir, connection: aliceConnection(), watch: false });
    await app.start();

    await writeFile(join(syncDir, userPath), '# user file\n', 'utf8');
    await app.pushAllLocalFiles();

    const files = await app.listLocalFiles();
    assert.equal(files.some((file) => file.startsWith('.index/')), false);
    assert.equal(files.includes(userPath), true);

    const { client } = await login('alice', 'alice123');
    const rows = await client.queryAll(`SELECT path FROM okf_sync.context_files WHERE path LIKE $1`, ['.index%']);
    assert.equal(rows.length, 0);
    await client.disconnect();
  } finally {
    await stopSyncApp(app);
    await deletePaths(paths).catch(() => undefined);
    await rm(syncDir, { recursive: true, force: true });
  }
});
