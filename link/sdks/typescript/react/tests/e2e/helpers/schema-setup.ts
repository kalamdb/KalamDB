const KALAM_URL = process.env.KALAM_URL ?? 'http://127.0.0.1:2900';
const KALAM_USER = process.env.KALAM_USER ?? 'root';
const KALAM_PASSWORD = process.env.KALAM_PASSWORD ?? 'kalamdb123';

let cachedToken: string | null = null;

async function getToken(): Promise<string> {
  if (cachedToken) return cachedToken;
  const res = await fetch(`${KALAM_URL}/v1/api/auth/login`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ username: KALAM_USER, password: KALAM_PASSWORD }),
  });
  if (!res.ok) {
    throw new Error(`Login failed (${res.status}): ${await res.text().catch(() => '')}`);
  }
  const body = (await res.json()) as { access_token: string };
  cachedToken = body.access_token;
  return cachedToken;
}

async function runSql(sql: string): Promise<void> {
  const token = await getToken();
  const res = await fetch(`${KALAM_URL}/v1/api/sql`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', authorization: `Bearer ${token}` },
    body: JSON.stringify({ sql }),
  });
  if (!res.ok) {
    throw new Error(`SQL failed (${res.status}): ${sql}\n${await res.text().catch(() => '')}`);
  }
}

/**
 * Poll the table with `SELECT 1 ... LIMIT 1` until KalamDB's Raft consensus
 * has propagated the CREATE TABLE everywhere. Without this, subscriptions
 * spun up immediately after setupSchema() can hit NOT_FOUND.
 */
async function waitForTable(schemaName: string, tableName: string): Promise<void> {
  const maxAttempts = 20;
  const intervalMs = 200;
  let lastErr: unknown = null;
  for (let i = 0; i < maxAttempts; i++) {
    try {
      await runSql(`SELECT 1 FROM ${schemaName}.${tableName} LIMIT 1`);
      return;
    } catch (e) {
      lastErr = e;
      await new Promise((r) => setTimeout(r, intervalMs));
    }
  }
  throw lastErr;
}

const TEST_TABLES = ['messages', 'counters', 'encoded', 'composite'] as const;

export async function setupSchema(suffix: string): Promise<string> {
  const schemaName = `react_e2e_${suffix}`;
  await runSql(`DROP NAMESPACE IF EXISTS ${schemaName}`);
  await runSql(`CREATE NAMESPACE ${schemaName}`);
  await runSql(`CREATE TABLE ${schemaName}.messages (
    id TEXT PRIMARY KEY,
    room_id TEXT NOT NULL,
    body TEXT NOT NULL,
    author_name TEXT,
    created_at TIMESTAMP
  )`);
  await runSql(`CREATE TABLE ${schemaName}.counters (
    id TEXT PRIMARY KEY,
    value INTEGER NOT NULL,
    is_favorite BOOLEAN NOT NULL
  )`);
  await runSql(`CREATE TABLE ${schemaName}.encoded (
    id TEXT PRIMARY KEY,
    payload TEXT NOT NULL
  )`);
  await runSql(`CREATE TABLE ${schemaName}.composite (
    id TEXT PRIMARY KEY,
    room_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    value INTEGER NOT NULL
  )`);
  for (const table of TEST_TABLES) {
    await waitForTable(schemaName, table);
  }
  return schemaName;
}

export async function teardownSchema(suffix: string): Promise<void> {
  await runSql(`DROP NAMESPACE IF EXISTS react_e2e_${suffix}`).catch(() => undefined);
}

export async function seedMessages(suffix: string, roomId: string, count: number): Promise<void> {
  const schemaName = `react_e2e_${suffix}`;
  for (let i = 0; i < count; i++) {
    await runSql(`INSERT INTO ${schemaName}.messages (id, room_id, body, created_at)
      VALUES ('seed-${i}-${suffix}', '${roomId}', 'seed-body-${i}', NOW())`);
  }
}

export async function seedCounters(suffix: string, count: number): Promise<void> {
  const schemaName = `react_e2e_${suffix}`;
  for (let i = 0; i < count; i++) {
    await runSql(`INSERT INTO ${schemaName}.counters (id, value, is_favorite)
      VALUES ('c-${i}-${suffix}', ${i}, ${i % 2 === 0})`);
  }
}

export async function seedComposite(suffix: string, roomId: string, count: number): Promise<void> {
  const schemaName = `react_e2e_${suffix}`;
  for (let i = 0; i < count; i++) {
    await runSql(`INSERT INTO ${schemaName}.composite (id, room_id, message_id, value)
      VALUES ('c-${roomId}-${i}', '${roomId}', 'msg-${i}', ${i})`);
  }
}

export async function insertMessage(suffix: string, roomId: string, body: string): Promise<void> {
  const schemaName = `react_e2e_${suffix}`;
  await runSql(`INSERT INTO ${schemaName}.messages (id, room_id, body, created_at)
    VALUES ('${crypto.randomUUID()}', '${roomId}', '${body}', NOW())`);
}
