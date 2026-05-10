/**
 * FILE datatype e2e tests — upload, select, and download roundtrips.
 *
 * Run: node --test --test-concurrency=1 tests/e2e/query/file-download.test.mjs
 */

import { after, before, describe, test } from 'node:test';
import assert from 'node:assert/strict';

import { Auth, createClient } from '../../../dist/src/index.js';
import {
  ADMIN_PASS,
  ADMIN_USER,
  SERVER_URL,
  dropTable,
  ensureNamespace,
  uniqueName,
} from '../helpers.mjs';

async function loginBasicClient(user, password) {
  const client = createClient({
    url: SERVER_URL,
    authProvider: async () => Auth.basic(user, password),
  });
  const login = await client.login();
  return {
    client,
    accessToken: login.access_token,
    userId: login.user.id,
  };
}

async function downloadText(url, accessToken) {
  const response = await fetchDownload(url, accessToken);

  if (!response.ok) {
    throw new Error(`Download failed (${response.status}): ${await response.text()}`);
  }

  const bytes = Buffer.from(await response.arrayBuffer());
  return {
    response,
    text: bytes.toString('utf8'),
  };
}

async function fetchDownload(url, accessToken) {
  return fetch(url, {
    headers: {
      Authorization: `Bearer ${accessToken}`,
    },
  });
}

describe('FILE download roundtrip', { timeout: 30_000 }, () => {
  let adminClient;
  let adminAccessToken;

  const ns = uniqueName('ts_file');
  const sharedTable = `${ns}.shared_files`;
  const userTable = `${ns}.user_files`;

  before(async () => {
    const adminSession = await loginBasicClient(ADMIN_USER, ADMIN_PASS);
    adminClient = adminSession.client;
    adminAccessToken = adminSession.accessToken;

    await ensureNamespace(adminClient, ns);
    await adminClient.query(
      `CREATE TABLE IF NOT EXISTS ${sharedTable} (
        id TEXT PRIMARY KEY,
        attachment FILE
      )`,
    );
    await adminClient.query(
      `CREATE TABLE IF NOT EXISTS ${userTable} (
        id TEXT PRIMARY KEY,
        attachment FILE
      ) WITH (TYPE = 'USER')`,
    );
  });

  after(async () => {
    await dropTable(adminClient, sharedTable);
    await dropTable(adminClient, userTable);
    await adminClient.disconnect();
  });

  test('shared table upload can be selected and downloaded again', async () => {
    const rowId = uniqueName('shared_row');
    const content = `shared-file-roundtrip:${rowId}`;

    await adminClient.queryWithFiles(
      `INSERT INTO ${sharedTable} (id, attachment) VALUES ($1, FILE("upload"))`,
      {
        upload: new File([content], 'shared-roundtrip.txt', { type: 'text/plain' }),
      },
      [rowId],
    );

    const rows = await adminClient.queryRows(
      `SELECT id, attachment FROM ${sharedTable} WHERE id = $1`,
      sharedTable,
      [rowId],
    );

    assert.equal(rows.length, 1);
    const attachment = rows[0].file('attachment');
    assert.ok(attachment !== null);
    assert.equal(
      attachment.relativeUrl(),
      `/v1/files/${ns}/shared_files/${attachment.sub}/${attachment.storedName()}`,
    );

    const downloaded = await downloadText(attachment.downloadUrl(), adminAccessToken);
    assert.equal(downloaded.text, content);
  });

  test('service execute-as upload lands in the target user table and the user can download it', async () => {
    const userName = uniqueName('file_user');
    const userPassword = 'FileUserPass123!';
    const serviceName = uniqueName('file_service');
    const servicePassword = 'FileServicePass123!';
    const rowId = uniqueName('agent_row');
    const content = `execute-as-file-roundtrip:${rowId}`;

    await adminClient.query(
      `CREATE USER '${userName}' WITH PASSWORD '${userPassword}' ROLE user`,
    );
    await adminClient.query(
      `CREATE USER '${serviceName}' WITH PASSWORD '${servicePassword}' ROLE service`,
    );

    const userSession = await loginBasicClient(userName, userPassword);
    const serviceSession = await loginBasicClient(serviceName, servicePassword);

    try {
      await serviceSession.client.queryWithFiles(
        `EXECUTE AS USER '${userSession.userId}' (INSERT INTO ${userTable} (id, attachment) VALUES ($1, FILE("upload")))`,
        {
          upload: new File([content], 'agent-roundtrip.txt', { type: 'text/plain' }),
        },
        [rowId],
      );

      const serviceRows = await serviceSession.client.queryRows(
        `SELECT id, attachment FROM ${userTable} WHERE id = $1`,
        userTable,
        [rowId],
      );
      assert.equal(serviceRows.length, 0);

      const userRows = await userSession.client.queryRows(
        `SELECT id, attachment FROM ${userTable} WHERE id = $1`,
        userTable,
        [rowId],
      );

      assert.equal(userRows.length, 1);
      const attachment = userRows[0].file('attachment');
      assert.ok(attachment !== null);

      const serviceDirectDownload = await fetchDownload(
        `${attachment.downloadUrl()}?user_id=${encodeURIComponent(userSession.userId)}`,
        serviceSession.accessToken,
      );
      assert.equal(serviceDirectDownload.status, 403);

      const downloaded = await downloadText(attachment.downloadUrl(), userSession.accessToken);
      assert.equal(downloaded.text, content);
    } finally {
      await serviceSession.client.disconnect();
      await userSession.client.disconnect();
    }
  });
});