/**
 * Shared helpers for the OKF folder sync example.
 *
 * Small, reusable utilities live here so sync-engine.ts stays focused on orchestration.
 */

import { mkdir, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import type { QueryResponse } from '@kalamdb/client';
import { createKalamClient, resolveKalamConnection } from './db/client.js';
import { sha256Hex } from './lib/file-utils.js';
import { projectRoot, syncFilePath, syncParentDir } from './lib/paths.js';
import { resolveRootPassword } from './lib/server-credentials.js';

type SqlClient = {
  query: (sql: string) => Promise<QueryResponse>;
};

function sqlFailureMessage(error: unknown, response?: QueryResponse): string {
  if (response?.error?.message) {
    return response.error.details
      ? `${response.error.message}: ${response.error.details}`
      : response.error.message;
  }
  if (error instanceof Error) {
    return error.message;
  }
  return String(error ?? 'unknown SQL error');
}

function isAlreadyExists(message: string): boolean {
  return /already exists|duplicate|conflict|idempotent/i.test(message);
}

async function runSql(client: SqlClient, sql: string): Promise<void> {
  const response = await client.query(sql);
  if (response.status === 'error' || response.error) {
    throw new Error(sqlFailureMessage(undefined, response));
  }
}

/** Create a password user, or reset the password if that user already exists. */
export async function ensureUser(client: SqlClient, name: string, password: string): Promise<void> {
  try {
    await runSql(
      client,
      `CREATE USER '${name}' WITH PASSWORD '${password}' ROLE 'user'`,
    );
  } catch (error) {
    if (!isAlreadyExists(sqlFailureMessage(error))) {
      throw error;
    }
    await runSql(client, `ALTER USER '${name}' SET PASSWORD '${password}'`);
  }
}

/** Pause polling loops used by test wait helpers. */
export function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/** Run async tasks one at a time (folder watcher + live subscription). */
export class TaskQueue {
  private tail: Promise<void> = Promise.resolve();

  enqueue(work: () => Promise<void>): void {
    this.tail = this.tail.then(work).catch((error) => {
      console.error('[sync] background task failed:', error);
    });
  }

  async waitIdle(): Promise<void> {
    await this.tail.catch(() => undefined);
  }
}

let demoBootstrap: Promise<void> | null = null;

async function applyOkfSchema(client: SqlClient): Promise<void> {
  const schema = await readFile(join(projectRoot(), 'kalam/schema.sql'), 'utf8');
  // Run one statement at a time — multi-statement SQL can stop after CREATE NAMESPACE
  // and leave context_files missing, which breaks the first integration SELECT.
  const statements = schema
    .split(';')
    .map((statement) => statement.trim())
    .filter((statement) => statement.length > 0);
  for (const statement of statements) {
    try {
      await runSql(client, statement);
    } catch (error) {
      if (!isAlreadyExists(sqlFailureMessage(error))) {
        throw error;
      }
    }
  }
}

/**
 * Ensure `okf_sync.context_files` and demo users exist.
 *
 * Called from FolderSyncApp.start so integration tests and `npm run dev`
 * work against a bare server without a separate migrate step.
 */
export async function ensureDemoUsers(): Promise<void> {
  if (!demoBootstrap) {
    demoBootstrap = (async () => {
      const connection = resolveKalamConnection({
        ...process.env,
        KALAM_USER: 'root',
        KALAM_PASSWORD: resolveRootPassword(),
      });
      const client = createKalamClient(connection);
      await client.initialize();
      try {
        await applyOkfSchema(client);
        for (const [name, password] of [['alice', 'alice123'], ['bob', 'bob123']] as const) {
          await ensureUser(client, name, password);
        }
      } finally {
        await client.disconnect();
      }
    })().catch((error) => {
      demoBootstrap = null;
      throw error;
    });
  }
  await demoBootstrap;
}

export async function readSyncFileBytes(syncDir: string, relativePath: string): Promise<Uint8Array> {
  return new Uint8Array(await readFile(syncFilePath(syncDir, relativePath)));
}

export async function readSyncFileHash(syncDir: string, relativePath: string): Promise<string | null> {
  try {
    return sha256Hex(await readSyncFileBytes(syncDir, relativePath));
  } catch {
    return null;
  }
}

export async function writeSyncFile(
  syncDir: string,
  relativePath: string,
  bytes: Uint8Array,
): Promise<void> {
  await writeFile(syncFilePath(syncDir, relativePath), bytes);
}

export async function removeSyncFile(syncDir: string, relativePath: string): Promise<void> {
  try {
    await rm(syncFilePath(syncDir, relativePath), { force: true });
  } catch {
    // File may already be gone.
  }
}

export async function ensureSyncParentDir(syncDir: string, relativePath: string): Promise<void> {
  await mkdir(syncParentDir(syncDir, relativePath), { recursive: true });
}

export async function waitForLocalFiles(
  syncDir: string,
  paths: string[],
  timeoutMs = 15_000,
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const missing: string[] = [];
    for (const path of paths) {
      try {
        await stat(syncFilePath(syncDir, path));
      } catch {
        missing.push(path);
      }
    }
    if (missing.length === 0) {
      return;
    }
    await sleep(100);
  }
  throw new Error(`timed out waiting for files: ${paths.join(', ')}`);
}

export async function waitForLocalFileAbsent(
  syncDir: string,
  relativePath: string,
  timeoutMs = 15_000,
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      await stat(syncFilePath(syncDir, relativePath));
    } catch {
      return;
    }
    await sleep(100);
  }
  throw new Error(`timed out waiting for ${relativePath} to be removed`);
}
