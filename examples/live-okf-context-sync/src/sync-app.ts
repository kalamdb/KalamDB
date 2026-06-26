#!/usr/bin/env node
/**
 * OKF folder sync worker.
 *
 *   local disk  --push-->  KalamDB   (folder watcher + pending queue)
 *   KalamDB     --pull-->  local disk (live subscription)
 */

import 'dotenv/config';
import { readFile } from 'node:fs/promises';
import { pathToFileURL } from 'node:url';
import { liveTable } from '@kalamdb/orm';
import { createDb, createKalamClient, resolveKalamConnection, type KalamConnection } from './db/client.js';
import { openLocalDb } from './db/local-db.js';
import { watchSyncFolder, type FolderWatcher } from './folder-watcher.js';
import {
  ensureDemoUsers,
  ensureSyncParentDir,
  readSyncFileBytes,
  readSyncFileHash,
  removeSyncFile,
  TaskQueue,
  waitForLocalFileAbsent,
  waitForLocalFiles,
  writeSyncFile,
} from './helpers.js';
import { isSafeSyncPath, listSyncFiles, resolveSyncDir, syncDbPath, syncFilePath } from './lib/paths.js';
import { asFileRef } from './lib/file-utils.js';
import { maybeSeedSyncFolder } from './lib/seed.js';
import {
  clearPendingUpload,
  hasLocalRecord,
  listPendingUploads,
  listTrackedPaths,
  queuePendingUpload,
  readLocalSyncState,
  recordSyncedFile,
  removeLocalRecord,
} from './local-cache.js';
import { context_files, type ContextFiles } from './models/schema.generated.js';
import { toSyncFileRecord } from './models/schema.local.js';
import {
  deleteRemoteFile,
  downloadFileBytes,
  guessMimeType,
  remoteContentHash,
  sha256Hex,
  upsertSyncFile,
} from './remote-files.js';
import {
  InitialSyncTracker,
  logFileAdded,
  logFileDeleted,
  logFileDownloaded,
  logFilePushed,
  logFileUpdated,
  logInitialSyncCompleted,
  logInitialSyncStarted,
} from './sync-log.js';

export type FolderSyncOptions = {
  syncDir: string;
  connection: KalamConnection;
  watch?: boolean;
};

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
  private liveQueue = new TaskQueue();
  private liveUnsub: (() => Promise<void>) | null = null;
  private folderWatcher: FolderWatcher | null = null;
  private initialSyncTracker: InitialSyncTracker | null = null;

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

    const initialSync = new InitialSyncTracker();
    this.initialSyncTracker = initialSync;
    logInitialSyncStarted(this.syncDir);

    if (this.watchEnabled) {
      this.folderWatcher = await watchSyncFolder(
        this.syncDir,
        {
          onUpsert: (path) => this.pushLocalFile(path),
          onDelete: (path) => this.deleteRemoteFile(path, { log: false }),
        },
        { initialSync },
      );
      await this.reconcileLocalDeletions(initialSync);
    } else {
      await this.pushAllLocalFiles(initialSync);
      await this.reconcileLocalDeletions(initialSync);
    }

    await this.flushPendingUploads();
    logInitialSyncCompleted(this.syncDir, initialSync);
    this.initialSyncTracker = null;

    await this.startLivePull();
  }

  async stop(): Promise<void> {
    if (this.folderWatcher) {
      await this.folderWatcher.close();
      this.folderWatcher = null;
    }
    if (this.liveUnsub) {
      await this.liveUnsub();
      this.liveUnsub = null;
    }
    await this.liveQueue.waitIdle();
    await this.client.disconnect();
  }

  async listLocalFiles(): Promise<string[]> {
    return listSyncFiles(this.syncDir);
  }

  async readLocalFile(relativePath: string): Promise<string> {
    return readFile(syncFilePath(this.syncDir, relativePath), 'utf8');
  }

  waitForLocalFiles(paths: string[], timeoutMs = 15_000): Promise<void> {
    return waitForLocalFiles(this.syncDir, paths, timeoutMs);
  }

  waitForLocalFileAbsent(relativePath: string, timeoutMs = 15_000): Promise<void> {
    return waitForLocalFileAbsent(this.syncDir, relativePath, timeoutMs);
  }

  private markIgnoring(path: string, ms = 500): void {
    this.ignoringPaths.add(path);
    setTimeout(() => this.ignoringPaths.delete(path), ms);
  }

  private notePush(): void {
    if (this.initialSyncTracker) {
      this.initialSyncTracker.pushed += 1;
    }
  }

  async pushLocalFile(relativePath: string): Promise<void> {
    if (!isSafeSyncPath(relativePath) || this.ignoringPaths.has(relativePath)) {
      return;
    }

    const { cachedHash, hasPending } = await readLocalSyncState(this.localDb, relativePath);
    const bytes = await readSyncFileBytes(this.syncDir, relativePath);
    const hash = sha256Hex(bytes);

    if (!hasPending && cachedHash === hash) {
      return;
    }

    try {
      await upsertSyncFile(this.kalamDb, {
        path: relativePath,
        fileBytes: bytes,
        mimeType: guessMimeType(relativePath),
      });
      await recordSyncedFile(this.localDb, relativePath, hash);
      await clearPendingUpload(this.localDb, relativePath);
      this.remotePaths.add(relativePath);
      logFilePushed(relativePath, bytes.byteLength);
      this.notePush();
    } catch (error) {
      await queuePendingUpload(this.localDb, relativePath, hash, error);
      throw error;
    }
  }

  async pushAllLocalFiles(initialSync?: InitialSyncTracker): Promise<void> {
    for (const path of await listSyncFiles(this.syncDir)) {
      const hadRow = await hasLocalRecord(this.localDb, path);
      const pushedBefore = initialSync?.pushed ?? 0;

      try {
        await this.pushLocalFile(path);
      } catch {
        // Individual failures are queued in SQLite.
      }

      if (initialSync && initialSync.pushed > pushedBefore) {
        if (hadRow) {
          logFileUpdated(path);
          initialSync.updated += 1;
        } else {
          logFileAdded(path);
          initialSync.added += 1;
        }
      }
    }
  }

  private async reconcileLocalDeletions(initialSync?: InitialSyncTracker): Promise<void> {
    const onDisk = new Set(await listSyncFiles(this.syncDir));

    for (const path of await listTrackedPaths(this.localDb)) {
      if (onDisk.has(path)) {
        continue;
      }

      logFileDeleted(path);
      if (initialSync) {
        initialSync.deleted += 1;
      }

      try {
        await this.deleteRemoteFile(path, { log: false });
      } catch (error) {
        console.error(`[sync] failed to delete remote ${path}:`, error);
      }
    }
  }

  async flushPendingUploads(): Promise<void> {
    for (const row of await listPendingUploads(this.localDb)) {
      try {
        await this.pushLocalFile(row.path);
      } catch {
        // Remains queued for the next reconnect.
      }
    }
  }

  async deleteRemoteFile(relativePath: string, options: { log?: boolean } = {}): Promise<void> {
    if (!isSafeSyncPath(relativePath)) {
      return;
    }

    if (options.log ?? true) {
      logFileDeleted(relativePath);
      if (this.initialSyncTracker) {
        this.initialSyncTracker.deleted += 1;
      }
    }

    await deleteRemoteFile(this.kalamDb, relativePath);
    await removeLocalRecord(this.localDb, relativePath);
    await clearPendingUpload(this.localDb, relativePath);
    this.remotePaths.delete(relativePath);
  }

  private async pullRemoteRow(row: ContextFiles): Promise<void> {
    const relativePath = row.path;
    if (!isSafeSyncPath(relativePath)) {
      console.warn(`[sync] ignoring remote row with unsafe path: ${relativePath}`);
      return;
    }

    const fileRef = asFileRef(row.file_ref);
    const remoteHash = remoteContentHash(fileRef);
    if (!fileRef || !remoteHash) {
      console.warn(`[sync] skipping ${relativePath}: missing file_ref.sha256`);
      return;
    }

    const localHash = await readSyncFileHash(this.syncDir, relativePath);
    if (localHash === remoteHash) {
      await recordSyncedFile(
        this.localDb,
        relativePath,
        remoteHash,
        row.updated_at ?? row.created_at ?? new Date(),
      );
      return;
    }

    const bytes = await downloadFileBytes(this.connection.url, fileRef, this.accessToken);
    await ensureSyncParentDir(this.syncDir, relativePath);
    this.markIgnoring(relativePath);
    await writeSyncFile(this.syncDir, relativePath, bytes);

    const record = toSyncFileRecord({
      path: relativePath,
      sha256: remoteHash,
      created_at: row.created_at,
      updated_at: row.updated_at,
    });
    await recordSyncedFile(this.localDb, record.path, record.sha256, record.updated_at);
    logFileDownloaded(relativePath, bytes.byteLength);
  }

  private async removeLocalFile(relativePath: string): Promise<void> {
    if (!isSafeSyncPath(relativePath)) {
      return;
    }

    this.markIgnoring(relativePath);
    await removeSyncFile(this.syncDir, relativePath);
    await removeLocalRecord(this.localDb, relativePath);
    this.remotePaths.delete(relativePath);
    logFileDeleted(relativePath);
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
    this.liveUnsub = await liveTable(this.client, context_files, (rows) => {
      this.liveQueue.enqueue(() => this.handleLiveRows(rows));
    });
    console.log('[sync] live subscription active');
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
