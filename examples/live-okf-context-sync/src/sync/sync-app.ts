#!/usr/bin/env node
import 'dotenv/config';
import { mkdir, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { watch } from 'node:fs';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { liveTable } from '@kalamdb/orm';
import { eq } from 'drizzle-orm';
import { createDb, createKalamClient, resolveKalamConnection, type KalamConnection } from '../db/client.js';
import { openLocalDb } from '../db/local-db.js';
import {
  isExcludedWatchPath,
  isSafeSyncPath,
  listSyncFiles,
  resolveSyncDir,
  syncDbPath,
} from '../lib/paths.js';
import { resolveRootPassword } from '../lib/server-credentials.js';
import { context_files, type ContextFiles } from '../models/schema.generated.js';
import { local_context_files, pending_uploads, toSyncFileRecord } from '../models/schema.local.js';
import {
  downloadFileBytes,
  guessMimeType,
  remoteContentHash,
  sha256Hex,
  upsertFile,
} from './file-store.js';
import { maybeSeedSyncFolder } from './seed.js';

export type FolderSyncOptions = {
  syncDir: string;
  connection: KalamConnection;
  watch?: boolean;
};

function debounce<T extends (...args: never[]) => void>(fn: T, ms: number): T {
  let timer: NodeJS.Timeout | undefined;
  return ((...args: Parameters<T>) => {
    clearTimeout(timer);
    timer = setTimeout(() => fn(...args), ms);
  }) as T;
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function ensureDemoUsers(): Promise<void> {
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

export class FolderSyncApp {
  readonly syncDir: string;
  private readonly connection: KalamConnection;
  private readonly watchEnabled: boolean;
  private readonly localDb;
  private client: ReturnType<typeof createKalamClient>;
  private kalamDb: ReturnType<typeof createDb>;
  private accessToken = '';
  private remotePaths = new Set<string>();
  private ignoringPaths = new Set<string>();
  private liveQueue: Promise<void> = Promise.resolve();
  private liveUnsub: (() => Promise<void>) | null = null;

  constructor(options: FolderSyncOptions) {
    this.syncDir = options.syncDir;
    this.connection = options.connection;
    this.watchEnabled = options.watch ?? true;
    this.localDb = openLocalDb(syncDbPath(this.syncDir));
    this.client = createKalamClient(this.connection);
    this.kalamDb = createDb(this.client);
  }

  async start(): Promise<void> {
    await ensureDemoUsers();
    await this.client.initialize();
    const login = await this.client.login();
    this.accessToken = login.access_token;
    this.kalamDb = createDb(this.client);

    const remoteRows = await this.kalamDb.select({ path: context_files.path }).from(context_files);
    this.remotePaths = new Set(remoteRows.map((row) => row.path));
    const seeded = await maybeSeedSyncFolder(this.syncDir, remoteRows.length > 0);
    if (seeded > 0) {
      console.log(`[sync] seeded ${seeded} file(s) from seed/ into ${this.syncDir}`);
    }

    this.client.onConnect(() => {
      void this.flushPendingUploads().catch((error) => {
        console.error('[sync] failed to flush pending uploads after reconnect:', error);
      });
    });

    console.log(`[sync] watching ${this.syncDir} as ${this.connection.user}`);
    await this.pushAllLocalFiles();
    await this.flushPendingUploads();
    await this.startLivePull();

    if (this.watchEnabled) {
      this.startFolderWatch();
    }
  }

  async stop(): Promise<void> {
    if (this.liveUnsub) {
      await this.liveUnsub();
      this.liveUnsub = null;
    }
    await this.liveQueue.catch(() => undefined);
    await this.client.disconnect();
  }

  async listLocalFiles(): Promise<string[]> {
    return listSyncFiles(this.syncDir);
  }

  async readLocalFile(relativePath: string): Promise<string> {
    return readFile(join(this.syncDir, relativePath), 'utf8');
  }

  async waitForLocalFiles(paths: string[], timeoutMs = 15_000): Promise<void> {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      const missing: string[] = [];
      for (const path of paths) {
        try {
          await stat(join(this.syncDir, path));
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

  async waitForLocalFileAbsent(relativePath: string, timeoutMs = 15_000): Promise<void> {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      try {
        await stat(join(this.syncDir, relativePath));
      } catch {
        return;
      }
      await sleep(100);
    }
    throw new Error(`timed out waiting for ${relativePath} to be removed`);
  }

  private markIgnoring(path: string, ms = 500): void {
    this.ignoringPaths.add(path);
    setTimeout(() => this.ignoringPaths.delete(path), ms);
  }

  private async recordLocalFile(path: string, sha256: string, at = new Date()): Promise<void> {
    await this.localDb
      .insert(local_context_files)
      .values({ path, sha256, created_at: at, updated_at: at })
      .onConflictDoUpdate({
        target: local_context_files.path,
        set: { sha256, updated_at: at },
      });
  }

  private async queuePendingUpload(path: string, sha256: string, error: unknown): Promise<void> {
    const now = new Date();
    await this.localDb
      .insert(pending_uploads)
      .values({ path, sha256, updated_at: now, last_error: String(error) })
      .onConflictDoUpdate({
        target: pending_uploads.path,
        set: { sha256, updated_at: now, last_error: String(error) },
      });
    console.warn(`[sync] queued pending upload for ${path}`);
  }

  private async clearPendingUpload(path: string): Promise<void> {
    await this.localDb.delete(pending_uploads).where(eq(pending_uploads.path, path));
  }

  async pushLocalFile(relativePath: string): Promise<void> {
    if (!isSafeSyncPath(relativePath)) {
      return;
    }

    const fullPath = join(this.syncDir, relativePath);
    const bytes = new Uint8Array(await readFile(fullPath));
    const hash = sha256Hex(bytes);
    const now = new Date();

    const localRow = await this.localDb
      .select()
      .from(local_context_files)
      .where(eq(local_context_files.path, relativePath));
    const pendingRow = await this.localDb
      .select()
      .from(pending_uploads)
      .where(eq(pending_uploads.path, relativePath));

    // Skip only when local cache matches, the server was last known good, and
    // there is nothing waiting in the pending queue.
    if (pendingRow.length === 0 && localRow[0]?.sha256 === hash) {
      return;
    }

    try {
      await upsertFile(this.client, {
        path: relativePath,
        fileBytes: bytes,
        mimeType: guessMimeType(relativePath),
      });
      await this.recordLocalFile(relativePath, hash, now);
      await this.clearPendingUpload(relativePath);
      this.remotePaths.add(relativePath);
      console.log(`[sync] pushed ${relativePath}`);
    } catch (error) {
      await this.queuePendingUpload(relativePath, hash, error);
      throw error;
    }
  }

  async pushAllLocalFiles(): Promise<void> {
    const files = await listSyncFiles(this.syncDir);
    for (const path of files) {
      try {
        await this.pushLocalFile(path);
      } catch {
        // Individual failures are queued in SQLite.
      }
    }
    console.log(`[sync] scanned ${files.length} local file(s)`);
  }

  async flushPendingUploads(): Promise<void> {
    const pending = await this.localDb.select().from(pending_uploads);
    for (const row of pending) {
      try {
        await this.pushLocalFile(row.path);
      } catch {
        // Remains queued for the next reconnect.
      }
    }
  }

  private async pullRemoteRow(row: ContextFiles): Promise<void> {
    const relativePath = row.path;
    if (!isSafeSyncPath(relativePath)) {
      console.warn(`[sync] ignoring remote row with unsafe path: ${relativePath}`);
      return;
    }

    const fileRef = row.file_ref;
    const remoteHash = remoteContentHash(fileRef);
    if (!fileRef || !remoteHash) {
      console.warn(`[sync] skipping ${relativePath}: missing file_ref.sha256`);
      return;
    }

    const fullPath = join(this.syncDir, relativePath);
    let localHash: string | null = null;
    try {
      localHash = sha256Hex(new Uint8Array(await readFile(fullPath)));
    } catch {
      localHash = null;
    }

    if (localHash === remoteHash) {
      await this.recordLocalFile(
        relativePath,
        remoteHash,
        row.updated_at ?? row.created_at ?? new Date(),
      );
      return;
    }

    // Download using the FileRef from this exact live event so the URL and the
    // verified hash always describe the same version.
    const bytes = await downloadFileBytes(this.connection.url, fileRef, this.accessToken);

    await mkdir(join(this.syncDir, relativePath.split('/').slice(0, -1).join('/')), { recursive: true });
    this.markIgnoring(relativePath);
    await writeFile(fullPath, bytes);

    const record = toSyncFileRecord({
      path: relativePath,
      sha256: remoteHash,
      created_at: row.created_at,
      updated_at: row.updated_at,
    });
    await this.recordLocalFile(record.path, record.sha256, record.updated_at);
    console.log(`[sync] pulled ${relativePath}`);
  }

  private async removeLocalFile(relativePath: string): Promise<void> {
    if (!isSafeSyncPath(relativePath)) {
      return;
    }
    const fullPath = join(this.syncDir, relativePath);
    this.markIgnoring(relativePath);
    try {
      await rm(fullPath, { force: true });
    } catch {
      // File may already be gone.
    }
    await this.localDb.delete(local_context_files).where(eq(local_context_files.path, relativePath));
    this.remotePaths.delete(relativePath);
    console.log(`[sync] removed ${relativePath}`);
  }

  private async handleLiveRows(rows: ContextFiles[]): Promise<void> {
    const nextPaths = new Set(rows.map((row) => row.path));

    for (const path of this.remotePaths) {
      if (!nextPaths.has(path)) {
        await this.removeLocalFile(path);
      }
    }
    this.remotePaths = nextPaths;

    for (const row of rows) {
      try {
        await this.pullRemoteRow(row);
      } catch (error) {
        console.error(`[sync] failed to pull ${row.path}:`, error);
      }
    }
  }

  private async startLivePull(): Promise<void> {
    // Live callbacks can arrive faster than a pull completes. Chain them so
    // only one snapshot is reconciled at a time, otherwise overlapping pulls and
    // pushes can read a FileRef from one version while writing another.
    this.liveUnsub = await liveTable(this.client, context_files, (rows) => {
      this.liveQueue = this.liveQueue
        .then(() => this.handleLiveRows(rows))
        .catch((error) => {
          console.error('[sync] live handler failed:', error);
        });
    });
    console.log('[sync] live subscription active');
  }

  private startFolderWatch(): void {
    const schedulePush = debounce((relativePath: string) => {
      if (this.ignoringPaths.has(relativePath)) {
        return;
      }
      void this.pushLocalFile(relativePath).catch((error) => {
        console.error(`[sync] failed to push ${relativePath}:`, error);
      });
    }, 250);

    watch(this.syncDir, { recursive: true }, (_event, filename) => {
      if (!filename || isExcludedWatchPath(filename)) {
        return;
      }

      const relativePath = filename.replaceAll('\\', '/');
      const fullPath = join(this.syncDir, relativePath);
      void stat(fullPath)
        .then((info) => {
          if (info.isFile()) {
            schedulePush(relativePath);
          }
        })
        .catch(() => {
          void this.removeLocalFile(relativePath).catch((error) => {
            console.error(`[sync] failed to remove ${relativePath}:`, error);
          });
        });
    });
  }
}

async function main(): Promise<void> {
  const app = new FolderSyncApp({
    syncDir: resolveSyncDir(process.argv[2]),
    connection: resolveKalamConnection(process.env),
  });
  await app.start();
}

const isMainModule = process.argv[1] !== undefined
  && import.meta.url === pathToFileURL(process.argv[1]).href;

if (isMainModule) {
  void main().catch((error) => {
    console.error('[sync] fatal error:', error);
    process.exit(1);
  });
}
