import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { Auth, createClient } from '@kalamdb/client';
import { createDb, TABLE } from '../src/client.js';
import { decideConflictAction } from '../src/conflicts.js';
import { downloadFileText, fetchServerSha256, upsertMetadata } from '../src/file-store.js';

const SERVER_URL = process.env.KALAM_URL ?? process.env.KALAMDB_URL ?? 'http://127.0.0.1:2900';
const ROOT_PASSWORD =
  process.env.KALAM_ROOT_PASSWORD
  ?? process.env.KALAMDB_PASSWORD
  ?? process.env.KALAM_PASS
  ?? 'kalamdb123';
const RUN_INTEGRATION = process.env.KALAM_INTEGRATION === '1';

async function ensureOkfSchema(root: Awaited<ReturnType<typeof login>>): Promise<void> {
  const schema = await readFile(resolve('kalam/schema.sql'), 'utf8');
  await root.client.query(schema);
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

test('integration: metadata roundtrip and isolation', { skip: !RUN_INTEGRATION }, async () => {
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

  try {
    await upsertMetadata(alice.client, {
      path,
      sha256: 'seed',
      baseSha256: null,
      mimeType: 'text/markdown',
      sizeBytes: bytes.byteLength,
      frontmatter: { title: 'Integration' },
      isConflict: false,
      canonicalPath: null,
      deleted: false,
      fileBytes: bytes,
    });

    const serverSha = await fetchServerSha256(aliceDb, path);
    assert.ok(serverSha);

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

    const text = await downloadFileText(aliceDb, SERVER_URL, path, alice.token);
    assert.match(text, /integration test/);

    await assert.rejects(
      () => downloadFileText(bobDb, SERVER_URL, path, bob.token),
      /missing file row/,
    );

    const decision = decideConflictAction({
      relativePath: path,
      localBaseSha256: serverSha,
      serverSha256: `${serverSha}-other`,
    });
    assert.equal(decision.kind, 'create-conflict');
  } finally {
    await alice.client.query(`DELETE FROM ${TABLE} WHERE path = $1`, [path]);
    await alice.client.disconnect();
    await bob.client.disconnect();
  }
});

test('schema.sql is the declared source of truth', async () => {
  const schema = await readFile(resolve('kalam/schema.sql'), 'utf8');
  assert.match(schema, /CREATE USER TABLE okf_sync\.context_files/);
  assert.match(schema, /file_ref FILE NOT NULL/);
  assert.doesNotMatch(schema, /schema\.ts/);
});
