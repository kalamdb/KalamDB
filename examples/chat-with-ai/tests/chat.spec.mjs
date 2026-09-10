import { existsSync, readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { test, expect } from '@playwright/test';

const exampleRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const serverUrl = process.env.KALAM_URL ?? process.env.KALAMDB_URL ?? 'http://127.0.0.1:2900';
const room = process.env.CHAT_TEST_ROOM ?? 'playwright-room-fallback';
const adminUsername = 'admin';
const adminPassword = 'kalamdb123';
const rootPassword = process.env.KALAMDB_ROOT_PASSWORD ?? adminPassword;

function sqlStatementsFromFile(sql) {
  return sql
    .replace(/^[ \t]*--[^\n]*\n?/gm, '')
    .split(/;\s*(?:\r?\n|$)/)
    .map((statement) => statement.trim())
    .filter(Boolean);
}

const chatSetupStatements = sqlStatementsFromFile(
  readFileSync(path.join(exampleRoot, 'kalam/schema.sql'), 'utf8'),
);

async function requestJson(url, {
  method = 'GET',
  headers,
  body,
} = {}) {
  const response = await fetch(url, {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await response.text();
  let json = null;

  if (text) {
    try {
      json = JSON.parse(text);
    } catch {
      json = null;
    }
  }

  return {
    ok: response.ok,
    status: response.status,
    text,
    json,
  };
}

async function isServerAvailable() {
  for (const healthUrl of [`${serverUrl}/health`, `${serverUrl}/v1/api/healthcheck`]) {
    try {
      const response = await fetch(healthUrl);
      if (response.ok) {
        return true;
      }
    } catch {
      // Try the fallback endpoint before treating the server as unavailable.
    }
  }

  return false;
}

function sqlResponseHasRows(body) {
  const result = Array.isArray(body?.results) ? body.results[0] : null;
  const rows = Array.isArray(result?.rows) ? result.rows.length : 0;
  const rowCount = typeof result?.row_count === 'number' ? result.row_count : rows;
  return rowCount > 0;
}

function isProviderNotReady(message) {
  return /provider not found/i.test(message);
}

async function login(user, password) {
  const result = await requestJson(`${serverUrl}/v1/api/auth/login`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: { user, password },
  });

  if (!result.ok) {
    throw new Error(`Login failed for ${user}: ${result.status}${result.text ? ` ${result.text}` : ''}`);
  }

  if (!result.json?.access_token) {
    throw new Error(`Login response for ${user} did not include an access token`);
  }

  return result.json.access_token;
}

async function executeSql(token, sql, { allowFailure = false } = {}) {
  const result = await requestJson(`${serverUrl}/v1/api/sql`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      Authorization: `Bearer ${token}`,
    },
    body: { sql },
  });

  if (!result.ok && !allowFailure) {
    throw new Error(`SQL failed: ${result.status} ${result.text}`);
  }

  return result;
}

async function ensureAdminReady() {
  const statusResult = await requestJson(`${serverUrl}/v1/api/auth/status`);
  if (statusResult.ok && statusResult.json?.needs_setup === true) {
    const setupResult = await requestJson(`${serverUrl}/v1/api/auth/setup`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: {
        user: adminUsername,
        password: adminPassword,
        root_password: rootPassword,
        email: null,
      },
    });

    if (!setupResult.ok) {
      throw new Error(`Auth setup failed: ${setupResult.status} ${setupResult.text}`);
    }

    return;
  }

  try {
    await login(adminUsername, adminPassword);
    return;
  } catch (adminError) {
    let rootToken;

    try {
      rootToken = await login('root', rootPassword);
    } catch (rootError) {
      throw new Error([
        `Admin login failed for ${serverUrl}.`,
        `Admin error: ${adminError instanceof Error ? adminError.message : String(adminError)}`,
        `Root error: ${rootError instanceof Error ? rootError.message : String(rootError)}`,
      ].join('\n'));
    }

    const userCheck = await executeSql(
      rootToken,
      `SELECT user_id FROM system.users WHERE user_id = ${sqlLiteral(adminUsername)} LIMIT 1`,
      { allowFailure: true },
    );

    const repairSql = sqlResponseHasRows(userCheck.json)
      ? `ALTER USER admin SET PASSWORD ${sqlLiteral(adminPassword)}; ALTER USER admin SET ROLE 'dba';`
      : `CREATE USER admin WITH PASSWORD ${sqlLiteral(adminPassword)} ROLE 'dba'`;
    const repairResult = await executeSql(rootToken, repairSql, { allowFailure: true });

    if (!repairResult.ok && !/already exists|duplicate|conflict|idempotent/i.test(repairResult.text)) {
      throw new Error(`Failed to prepare admin test user: ${repairResult.status} ${repairResult.text}`);
    }

    await login(adminUsername, adminPassword);
  }
}

async function prepareChatDemoSchema() {
  const token = await login(adminUsername, adminPassword);
  await executeSql(token, 'DROP TOPIC chat_demo.ai_inbox', { allowFailure: true });

  for (const statement of chatSetupStatements) {
    await executeSql(token, statement);
  }
}

async function ensureRoomAccess(userId, roomId) {
  const token = await login(adminUsername, adminPassword);
  await executeSql(
    token,
    `INSERT INTO chat_demo.rooms (id, title) VALUES (${sqlLiteral(roomId)}, ${sqlLiteral(roomId)})`,
    { allowFailure: true },
  );
  await executeSql(
    token,
    `INSERT INTO chat_demo.room_members (id, user_id, room_id) VALUES (${sqlLiteral(`${userId}:${roomId}`)}, ${sqlLiteral(userId)}, ${sqlLiteral(roomId)})`,
    { allowFailure: true },
  );
}

async function insertUserMessage(roomName, content, username = adminUsername) {
  await ensureRoomAccess(username, roomName);
  const token = await login(username, adminPassword);
  await executeSql(token, `CALL chat_demo.join_room(${sqlLiteral(roomName)})`);
  await executeSql(
    token,
    `CALL chat_demo.send_message('room', ${sqlLiteral(roomName)}, ${sqlLiteral(content)})`,
  );
}

function sqlLiteral(value) {
  return `'${value.replace(/'/g, "''")}'`;
}

function uniqueName(prefix) {
  return `${prefix}-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function waitFor(condition, {
  timeoutMs = 15_000,
  intervalMs = 50,
  description = 'condition',
} = {}) {
  const deadline = Date.now() + timeoutMs;

  while (Date.now() < deadline) {
    const result = await condition();
    if (result) {
      return result;
    }
    await sleep(intervalMs);
  }

  throw new Error(`Timed out waiting for ${description}`);
}

async function seedChatHistory() {
  await ensureRoomAccess(adminUsername, room);
  const token = await login('admin', 'kalamdb123');
  const values = Array.from({ length: 48 }, (_, index) => (
    `('${room}', 'assistant', 'KalamDB Copilot', 'admin', 'seed history ${index}-${Date.now()}')`
  )).join(', ');
  const statement = `INSERT INTO chat_demo.messages (room, role, author, sender_username, content) VALUES ${values}`;

  for (let attempt = 1; attempt <= 6; attempt += 1) {
    const result = await executeSql(token, statement, { allowFailure: true });
    if (result.ok) {
      return;
    }

    if (!isProviderNotReady(result.text) || attempt === 6) {
      throw new Error(`Failed to seed chat history: ${result.status} ${result.text}`);
    }

    await sleep(attempt * 250);
  }
}

function kalamBin() {
  const fromEnv = process.env.KALAM_BIN;
  if (fromEnv && existsSync(fromEnv)) {
    return fromEnv;
  }
  const debugBin = path.resolve(exampleRoot, '../../target/debug/kalam');
  if (existsSync(debugBin)) {
    return debugBin;
  }
  return 'kalam';
}

function queryCell(body, row = 0, column = 0) {
  return body?.results?.[0]?.rows?.[row]?.[column];
}

async function activateChatFunctions(token) {
  const modulePath = path.join(exampleRoot, 'functions/.kalam/build/module.js');
  const manifestPath = path.join(exampleRoot, 'functions/.kalam/build/manifest.json');
  const built = spawnSync(kalamBin(), ['functions', 'build'], {
    cwd: exampleRoot,
    encoding: 'utf8',
  });
  if (built.status !== 0) {
    throw new Error(
      `\`kalam functions build\` failed:\n${built.stdout}\n${built.stderr}`,
    );
  }
  if (!existsSync(modulePath) || !existsSync(manifestPath)) {
    throw new Error('functions/.kalam/build is missing after `kalam functions build`');
  }

  const artifact = readFileSync(modulePath, 'utf8');
  const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'));
  const exports = Object.entries(manifest.procedures ?? {})
    .filter(([, kind]) => kind === 'module')
    .map(([name]) => name);
  const result = await requestJson(
    `${serverUrl}/v1/api/functions/modules/${manifest.module ?? 'backend'}/activate`,
    {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        Authorization: `Bearer ${token}`,
      },
      body: {
        artifact,
        contractHash: manifest.contractHash,
        abiVersion: manifest.abiVersion ?? 2,
        exports,
      },
    },
  );
  if (!result.ok) {
    throw new Error(`function activate failed: ${result.status} ${result.text}`);
  }
}

async function waitForTriggerReady() {
  const token = await login(adminUsername, adminPassword);
  for (let attempt = 1; attempt <= 6; attempt += 1) {
    const probeRoom = uniqueName('trigger-ready');
    const probeMessage = `${probeRoom}-probe-${attempt}`;
    await insertUserMessage(probeRoom, probeMessage);
    try {
      await waitFor(async () => {
        const result = await executeSql(
          token,
          `SELECT content FROM chat_demo.messages WHERE room = ${sqlLiteral(probeRoom)} AND role = 'assistant' LIMIT 1`,
          { allowFailure: true },
        );
        const content = String(queryCell(result.json) ?? '');
        return content.includes('AI reply:');
      }, {
        timeoutMs: 15_000,
        description: `trigger readiness probe ${probeRoom}`,
      });
      return;
    } catch (error) {
      if (attempt === 6) {
        throw error;
      }
    }
  }
}

test.beforeAll(async () => {
  const available = await isServerAvailable();
  test.skip(!available, `KalamDB server is not reachable at ${serverUrl}`);

  await ensureAdminReady();
  await prepareChatDemoSchema();
  const token = await login(adminUsername, adminPassword);
  await activateChatFunctions(token);

  await waitForTriggerReady();
  await seedChatHistory();
});

test('message insert streams a live draft and syncs the committed reply to both tabs', async ({ browser, baseURL }) => {
  const uniqueMessage = `latency spike ${Date.now()}`;
  const context = await browser.newContext();
  const pageOne = await context.newPage();
  const pageTwo = await context.newPage();

  await pageOne.goto(baseURL);
  await pageTwo.goto(baseURL);

  await expect(pageOne.getByTestId('chat-status')).toContainText('Live');
  await expect(pageTwo.getByTestId('chat-status')).toContainText('Live');

  await pageOne.getByLabel('Message').fill(uniqueMessage);
  await pageOne.getByRole('button', { name: 'Send through KalamDB' }).click();

  await expect(pageOne.getByTestId('chat-thread')).toContainText(uniqueMessage);
  const userMessageContent = pageOne.getByTestId('chat-thread').getByText(uniqueMessage, { exact: true });
  await expect(userMessageContent).toBeVisible();
  const userMessage = userMessageContent.locator('xpath=ancestor::article[1]');
  await expect(userMessage.locator('header strong')).toHaveText('admin');
  await expect(userMessage.locator('header span')).toHaveText(/\d{1,2}:\d{2}(:\d{2})?\s?(AM|PM)?/);
  await expect(pageTwo.getByTestId('chat-thread')).toContainText(uniqueMessage);
  await expect(pageOne.getByTestId('stream-preview')).toContainText('KalamDB Copilot', { timeout: 15000 });
  await expect(pageOne.getByTestId('stream-preview')).toContainText('AI reply:', { timeout: 15000 });
  await expect(pageTwo.getByTestId('stream-preview')).toContainText('AI reply:', { timeout: 15000 });
  await expect(pageOne.getByTestId('agent-events')).toContainText('Assistant reply committed', { timeout: 30000 });
  await expect(pageOne.getByTestId('chat-thread')).toContainText('AI reply: KalamDB stored', { timeout: 30000 });
  await expect(pageTwo.getByTestId('chat-thread')).toContainText('AI reply: KalamDB stored', { timeout: 30000 });
  await expect(pageOne.getByTestId('stream-preview')).toHaveCount(0, { timeout: 30000 });
  await expect(pageTwo.getByTestId('stream-preview')).toHaveCount(0, { timeout: 30000 });

  const pageOneScroll = await pageOne.getByTestId('chat-thread').evaluate((node) => ({
    scrollTop: node.scrollTop,
    maxScrollTop: node.scrollHeight - node.clientHeight,
  }));
  const pageTwoScroll = await pageTwo.getByTestId('chat-thread').evaluate((node) => ({
    scrollTop: node.scrollTop,
    maxScrollTop: node.scrollHeight - node.clientHeight,
  }));

  expect(pageOneScroll.maxScrollTop - pageOneScroll.scrollTop).toBeLessThan(24);
  expect(pageTwoScroll.maxScrollTop - pageTwoScroll.scrollTop).toBeLessThan(24);
});

test('personal inbox sends through send_message and receives a trigger reply', async ({ browser, baseURL }) => {
  const uniqueMessage = `personal latency ${Date.now()}`;
  const context = await browser.newContext();
  const page = await context.newPage();

  await page.goto(baseURL);
  await expect(page.getByTestId('chat-status')).toContainText('Live');
  await page.getByTestId('chat-scope-direct').click();
  await expect(page.getByTestId('chat-status')).toContainText('Live');

  await page.getByLabel('Message').fill(uniqueMessage);
  await page.getByRole('button', { name: 'Send through KalamDB' }).click();

  await expect(page.getByTestId('chat-thread')).toContainText(uniqueMessage, { timeout: 15000 });
  await expect(page.getByTestId('stream-preview')).toContainText('AI reply:', { timeout: 15000 });
  await expect(page.getByTestId('chat-thread')).toContainText('AI reply: KalamDB stored', { timeout: 30000 });
  await expect(page.getByTestId('stream-preview')).toHaveCount(0, { timeout: 30000 });
});

test('a tab that joins mid-stream catches the active draft and final reply', async ({ browser, baseURL }) => {
  const uniqueMessage = `latency spike ${'x'.repeat(1200)} ${Date.now()}`;
  const context = await browser.newContext();
  const pageOne = await context.newPage();

  await pageOne.goto(baseURL);
  await expect(pageOne.getByTestId('chat-status')).toContainText('Live');

  await pageOne.getByLabel('Message').fill(uniqueMessage);
  await pageOne.getByRole('button', { name: 'Send through KalamDB' }).click();

  await expect(pageOne.getByTestId('stream-preview')).toContainText('AI reply:', { timeout: 15000 });

  const pageTwo = await context.newPage();
  await pageTwo.goto(baseURL);
  await expect(pageTwo.getByTestId('chat-status')).toContainText('Live');
  await expect(pageTwo.getByTestId('chat-thread')).toContainText(uniqueMessage, { timeout: 15000 });
  await expect(pageTwo.getByTestId('stream-preview')).toContainText('AI reply:', { timeout: 15000 });

  await pageTwo.reload();
  await expect(pageTwo.getByTestId('chat-status')).toContainText('Live');
  await expect(pageTwo.getByTestId('chat-thread')).toContainText(uniqueMessage, { timeout: 15000 });

  await expect(pageOne.getByTestId('chat-thread')).toContainText('AI reply: KalamDB stored', { timeout: 20000 });
  await expect(pageTwo.getByTestId('chat-thread')).toContainText('AI reply: KalamDB stored', { timeout: 20000 });
});

test('streaming still works when the user message contains an unmatched parenthesis', async ({ browser, baseURL }) => {
  const uniqueMessage = `manual repro ( ${Date.now()}`;
  const context = await browser.newContext();
  const pageOne = await context.newPage();
  const pageTwo = await context.newPage();

  await pageOne.goto(baseURL);
  await pageTwo.goto(baseURL);

  await expect(pageOne.getByTestId('chat-status')).toContainText('Live');
  await expect(pageTwo.getByTestId('chat-status')).toContainText('Live');

  await pageOne.getByLabel('Message').fill(uniqueMessage);
  await pageOne.getByRole('button', { name: 'Send through KalamDB' }).click();

  await expect(pageOne.getByTestId('chat-thread')).toContainText(uniqueMessage, { timeout: 15000 });
  await expect(pageTwo.getByTestId('chat-thread')).toContainText(uniqueMessage, { timeout: 15000 });
  await expect(pageOne.getByTestId('stream-preview')).toContainText('AI reply:', { timeout: 15000 });
  await expect(pageTwo.getByTestId('stream-preview')).toContainText('AI reply:', { timeout: 15000 });
  await expect(pageOne.getByTestId('chat-thread')).toContainText('AI reply: KalamDB stored', { timeout: 20000 });
  await expect(pageTwo.getByTestId('chat-thread')).toContainText('AI reply: KalamDB stored', { timeout: 20000 });
});
