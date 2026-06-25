import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { Auth, createClient } from '@kalamdb/client';
import { createKalamClient, resolveKalamConnection, TABLE } from '../src/db/client.js';

export const SERVER_URL = process.env.KALAM_URL ?? process.env.KALAMDB_URL ?? 'http://127.0.0.1:2900';
export const ROOT_PASSWORD =
  process.env.KALAM_ROOT_PASSWORD
  ?? process.env.KALAMDB_PASSWORD
  ?? process.env.KALAM_PASS
  ?? 'kalamdb123';
export const RUN_INTEGRATION = process.env.KALAM_INTEGRATION === '1';

export function aliceConnection() {
  return resolveKalamConnection({
    ...process.env,
    KALAM_URL: SERVER_URL,
    KALAM_USER: 'alice',
    KALAM_PASSWORD: 'alice123',
  });
}

export async function ensureOkfSchema(root: Awaited<ReturnType<typeof login>>): Promise<void> {
  const schema = await readFile(resolve('kalam/schema.sql'), 'utf8');
  try {
    await root.client.query(schema);
  } catch (error) {
    if (!/already exists/i.test(String(error))) {
      throw error;
    }
  }
}

export async function serverHealthy(): Promise<boolean> {
  try {
    const response = await fetch(`${SERVER_URL}/v1/api/auth/status`);
    return response.ok;
  } catch {
    return false;
  }
}

export async function login(user: string, password: string) {
  const client = createClient({
    url: SERVER_URL,
    namespace: 'okf_sync',
    authProvider: async () => Auth.basic(user, password),
    disableCompression: true,
  });
  const loginResult = await client.login();
  return { client, token: loginResult.access_token };
}

export async function deletePaths(paths: string[]): Promise<void> {
  const client = createKalamClient(aliceConnection());
  await client.initialize();
  await client.login();
  try {
    for (const path of paths) {
      await client.query(`DELETE FROM ${TABLE} WHERE path = $1`, [path]);
    }
  } finally {
    await client.disconnect();
  }
}

export function uniquePath(prefix: string): string {
  return `${prefix}-${Date.now()}-${Math.random().toString(16).slice(2, 8)}`;
}
