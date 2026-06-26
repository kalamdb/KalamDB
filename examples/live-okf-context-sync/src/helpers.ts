/**
 * Shared helpers for the OKF folder sync example.
 *
 * Small, reusable utilities live here so sync-app.ts stays focused on orchestration.
 */

import { mkdir, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { createKalamClient, resolveKalamConnection } from './db/client.js';
import { sha256Hex } from './lib/file-utils.js';
import { syncFilePath, syncParentDir } from './lib/paths.js';
import { resolveRootPassword } from './lib/server-credentials.js';

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

/** Create demo users once so the example works out of the box. */
export async function ensureDemoUsers(): Promise<void> {
  const connection = resolveKalamConnection({
    ...process.env,
    KALAM_USER: 'root',
    KALAM_PASSWORD: resolveRootPassword(),
  });
  const client = createKalamClient(connection);
  await client.initialize();

  for (const [name, password] of [['alice', 'alice123'], ['bob', 'bob123']] as const) {
    try {
      await client.query(`CREATE USER '${name}' WITH PASSWORD '${password}' ROLE 'user'`);
    } catch {
      // User already exists on subsequent runs.
    }
  }

  await client.disconnect();
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
