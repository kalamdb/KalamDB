import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { readFile as readSchema } from 'node:fs/promises';
import { resolve } from 'node:path';
import { Auth, createClient } from '@kalamdb/client';
import { createDb, createKalamClient, resolveKalamConnection, TABLE } from '../src/db/client.js';
import { downloadFileByPath, fetchRemoteHash, sha256Hex, upsertFile } from '../src/sync/file-store.js';
import { listSyncFiles } from '../src/lib/paths.js';
import { FolderSyncApp } from '../src/sync/sync-app.js';
import { stopSyncApp } from './sync.helpers.js';

const SERVER_URL = process.env.KALAM_URL ?? process.env.KALAMDB_URL ?? 'http://127.0.0.1:2900';
const ROOT_PASSWORD =
  process.env.KALAM_ROOT_PASSWORD
  ?? process.env.KALAMDB_PASSWORD
  ?? process.env.KALAM_PASS
  ?? 'kalamdb123';
const RUN_INTEGRATION = process.env.KALAM_INTEGRATION === '1';

async function ensureOkfSchema(root: Awaited<ReturnType<typeof login>>): Promise<void> {
  const schema = await readSchema(resolve('kalam/schema.sql'), 'utf8');
  try {
    await root.client.query(schema);
  } catch (error) {
    if (!/already exists/i.test(String(error))) {
      throw error;
    }
  }
}

async function serverHealthy(): Promise<boolean> {
  try {
    const response = await fetch(`${SERVER_URL}/v1/api/auth/status`);
    return response.ok;
  } catch {
    return false;
  }
}

async function login(user: string, password: string) {
  const client = createClient({
    url: SERVER_URL,
    namespace: 'okf_sync',
    authProvider: async () => Auth.basic(user, password),
    disableCompression: true,
  });
  const loginResult = await client.login();
  return { client, token: loginResult.access_token };
}

test('integration: file roundtrip and isolation', { skip: !RUN_INTEGRATION }, async () => {
  if (!(await serverHealthy())) {
    return;
  }

  const root = await login('root', ROOT_PASSWORD);
  await ensureOkfSchema(root);
  try {
    await root.client.query(`CREATE USER 'alice' WITH PASSWORD 'alice123' ROLE 'user'`);
  } catch {
    // already exists
  }
  try {
    await root.client.query(`CREATE USER 'bob' WITH PASSWORD 'bob123' ROLE 'user'`);
  } catch {
    // already exists
  }
  await root.client.disconnect();

  const alice = await login('alice', 'alice123');
  const bob = await login('bob', 'bob123');
  const aliceDb = createDb(alice.client);
  const bobDb = createDb(bob.client);

  const path = `integration-${Date.now()}.md`;
  const bytes = new TextEncoder().encode('# integration test\n');
  const hash = sha256Hex(bytes);

  try {
    await upsertFile(alice.client, {
      path,
      mimeType: 'text/markdown',
      fileBytes: bytes,
    });

    const serverHash = await fetchRemoteHash(aliceDb, path);
    assert.equal(serverHash, hash);

    const aliceRows = await alice.client.queryAll(
      `SELECT path FROM ${TABLE} WHERE path = $1`,
      [path],
    );
    assert.equal(aliceRows.length, 1);

    const bobRows = await bob.client.queryAll(
      `SELECT path FROM ${TABLE} WHERE path = $1`,
      [path],
    );
    assert.equal(bobRows.length, 0);

    const downloaded = await downloadFileByPath(aliceDb, SERVER_URL, path, alice.token);
    assert.equal(sha256Hex(downloaded), hash);

    await assert.rejects(
      () => downloadFileByPath(bobDb, SERVER_URL, path, bob.token),
      /missing file row/,
    );
  } finally {
    await alice.client.query(`DELETE FROM ${TABLE} WHERE path = $1`, [path]);
    await alice.client.disconnect();
    await bob.client.disconnect();
  }
});

test('integration: delete local folder and restore from database', { skip: !RUN_INTEGRATION }, async () => {
  if (!(await serverHealthy())) {
    return;
  }

  const root = await login('root', ROOT_PASSWORD);
  await ensureOkfSchema(root);
  await root.client.disconnect();

  const syncDir = await mkdtemp(join(tmpdir(), 'okf-resync-'));
  const testId = `resync-${Date.now()}`;
  const editedPath = `${testId}/edited.md`;
  const editedContent = `# edited ${testId}\nline two\n`;
  const connection = resolveKalamConnection({
    ...process.env,
    KALAM_URL: SERVER_URL,
    KALAM_USER: 'alice',
    KALAM_PASSWORD: 'alice123',
  });

  const cleanupClient = createKalamClient(connection);
  await cleanupClient.initialize();
  await cleanupClient.login();

  const expectedPaths: string[] = [];
  let first: FolderSyncApp | undefined;
  let second: FolderSyncApp | undefined;

  try {
    first = new FolderSyncApp({ syncDir, connection, watch: false });
    await first.start();

    await mkdir(join(syncDir, testId), { recursive: true });
    await writeFile(join(syncDir, editedPath), editedContent, 'utf8');
    await first.pushLocalFile(editedPath);

    expectedPaths.push(...await listSyncFiles(syncDir));
    const expectedContents = Object.fromEntries(
      await Promise.all(expectedPaths.map(async (path) => [path, await first!.readLocalFile(path)] as const)),
    );

    await stopSyncApp(first);
    first = undefined;
    await rm(syncDir, { recursive: true, force: true });

    second = new FolderSyncApp({ syncDir, connection, watch: false });
    await second.start();
    await second.waitForLocalFiles(expectedPaths, 20_000);

    for (const path of expectedPaths) {
      const restored = await second.readLocalFile(path);
      assert.equal(restored, expectedContents[path], `content mismatch for ${path}`);
    }
  } finally {
    await stopSyncApp(first);
    await stopSyncApp(second);
    for (const path of expectedPaths) {
      await cleanupClient.query(`DELETE FROM ${TABLE} WHERE path = $1`, [path]).catch(() => undefined);
    }
    await cleanupClient.disconnect().catch(() => undefined);
    await rm(syncDir, { recursive: true, force: true });
  }
});

test('schema.sql is the declared source of truth', async () => {
  const schema = await readSchema(resolve('kalam/schema.sql'), 'utf8');
  assert.match(schema, /CREATE USER TABLE okf_sync\.context_files/);
  assert.match(schema, /file_ref FILE NOT NULL/);
  assert.doesNotMatch(schema, /sha256 TEXT/);
  assert.doesNotMatch(schema, /schema\.ts/);
});
