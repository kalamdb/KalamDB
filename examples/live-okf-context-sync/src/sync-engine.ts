/**
 * OKF folder sync engine.
 *
 *   local disk  --push-->  KalamDB   (folder watcher + pending queue)
 *   KalamDB     --pull-->  local disk (live subscription)
 */

import { liveTable } from '@kalamdb/orm';
import { createDb, createKalamClient, type KalamConnection } from './db/client.js';
import { openLocalDb } from './db/local-db.js';
import { watchSyncFolder, type FolderWatcher } from './folder-watcher.js';
import {
  ensureDemoUsers,
  ensureSyncParentDir,
  readSyncFileBytes,
  readSyncFileHash,
  removeSyncFile,
  TaskQueue,
  writeSyncFile,
} from './helpers.js';
import { isSafeSyncPath, listSyncFiles, syncDbPath } from './lib/paths.js';
import { SyncTombstones } from './lib/sync-tombstones.js';
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
  fetchRemoteFileVersion,
  guessMimeType,
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
  /** Paths with local edits being pushed; live pull must not overwrite these. */
  private localDirtyPaths = new Set<string>();
  private syncQueue = new TaskQueue();
  private tombstones = new SyncTombstones();
  private liveUnsub: (() => Promise<void>) | null = null;
  private folderWatcher: FolderWatcher | null = null;
  private initialSyncTracker: InitialSyncTracker | null = null;
  private liveSnapshotsHandled = 0;

  constructor(options: FolderSyncOptions) {
    this.syncDir = options.syncDir;
    this.connection = options.connection;
    this.watchEnabled = options.watch ?? true;
    this.localDb = openLocalDb(syncDbPath(this.syncDir));
    this.client = createKalamClient(this.connection);
    this.kalamDb = createDb(this.client);
  }

  async start(): Promise<void> {
    await this.connect();
    const remoteRows = await this.loadRemoteRows();
    await this.seedFolderIfServerEmpty(remoteRows);
    this.flushPendingUploadsOnReconnect();

    console.log(`[sync] watching ${this.syncDir} as ${this.connection.user}`);

    const initialSync = new InitialSyncTracker();
    this.initialSyncTracker = initialSync;
    logInitialSyncStarted(this.syncDir);

    try {
      await this.pullInitialRemoteFiles(remoteRows, initialSync);
      await this.startLocalPush(initialSync);
      await this.flushPendingUploads();
      logInitialSyncCompleted(this.syncDir, initialSync);
    } finally {
      this.initialSyncTracker = null;
    }

    await this.startLivePull();
  }

  private async connect(): Promise<void> {
    await ensureDemoUsers();
    await this.client.initialize();
    const login = await this.client.login();
    this.accessToken = login.access_token;
    this.kalamDb = createDb(this.client);
  }

  private async loadRemoteRows(): Promise<ContextFiles[]> {
    const remoteRows = await this.kalamDb.select().from(context_files);
    this.remotePaths = new Set(remoteRows.map((row) => row.path));
    return remoteRows;
  }

  private async seedFolderIfServerEmpty(remoteRows: ContextFiles[]): Promise<void> {
    const seeded = await maybeSeedSyncFolder(this.syncDir, remoteRows.length > 0);
    if (seeded > 0) {
      console.log(`[sync] seeded ${seeded} file(s) from seed/ into ${this.syncDir}`);
    }
  }

  private flushPendingUploadsOnReconnect(): void {
    this.client.onConnect(() => {
      void this.flushPendingUploads().catch((error) => {
        console.error('[sync] failed to flush pending uploads after reconnect:', error);
      });
    });
  }

  private async startLocalPush(initialSync: InitialSyncTracker): Promise<void> {
    if (this.watchEnabled) {
      this.folderWatcher = await watchSyncFolder(
        this.syncDir,
        {
          onUpsert: (path) => this.pushLocalFile(path),
          onDelete: (path) => {
            if (this.ignoringPaths.has(path)) {
              return Promise.resolve();
            }
            return this.deleteRemoteFile(path, { log: false });
          },
        },
        {
          initialSync,
          shouldSuppressEvent: (path) => this.ignoringPaths.has(path),
          taskQueue: this.syncQueue,
        },
      );
    } else {
      await this.pushAllLocalFiles(initialSync);
    }

    await this.reconcileLocalDeletions(initialSync);
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
    await this.syncQueue.waitIdle();
    await this.client.disconnect();
  }

  private markIgnoring(path: string, ms = 1_000): void {
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

    this.localDirtyPaths.add(relativePath);

    const { cachedHash, hasPending } = await readLocalSyncState(this.localDb, relativePath);
    const bytes = await readSyncFileBytes(this.syncDir, relativePath);
    const hash = sha256Hex(bytes);

    if (this.tombstones.shouldBlockLocalPush(relativePath, hash)) {
      await clearPendingUpload(this.localDb, relativePath);
      this.localDirtyPaths.delete(relativePath);
      return;
    }

    if (!hasPending && cachedHash === hash) {
      this.localDirtyPaths.delete(relativePath);
      return;
    }

    try {
      await upsertSyncFile(this.kalamDb, {
        path: relativePath,
        fileBytes: bytes,
        mimeType: guessMimeType(relativePath),
      });
      const remoteVersion = await this.fetchRemoteVersionForLog(relativePath);
      await recordSyncedFile(this.localDb, relativePath, hash);
      await clearPendingUpload(this.localDb, relativePath);
      this.remotePaths.add(relativePath);
      this.tombstones.clear(relativePath);
      logFilePushed({
        path: remoteVersion?.path ?? relativePath,
        seq: remoteVersion?.seq,
        sizeBytes: bytes.byteLength,
      });
      this.notePush();
      this.localDirtyPaths.delete(relativePath);
    } catch (error) {
      await queuePendingUpload(this.localDb, relativePath, hash, error);
      throw error;
    }
  }

  private async fetchRemoteVersionForLog(relativePath: string) {
    try {
      return await fetchRemoteFileVersion(this.client, relativePath);
    } catch {
      return null;
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

    const { cachedHash } = await readLocalSyncState(this.localDb, relativePath);
    const contentHash = cachedHash ?? await readSyncFileHash(this.syncDir, relativePath);
    this.tombstones.mark(relativePath, contentHash);

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

  private async pullInitialRemoteFiles(
    rows: ContextFiles[],
    initialSync: InitialSyncTracker,
  ): Promise<void> {
    if (rows.length === 0) {
      return;
    }

    for (const row of rows) {
      try {
        await this.pullRemoteRow(row, { skipLocalConflicts: true });
      } catch (error) {
        console.error(`[sync] failed to download ${row.path} during initial sync:`, error);
      }
    }

    if (initialSync.downloaded > 0) {
      console.log(`[sync] downloaded ${initialSync.downloaded} file(s) from server`);
    }
  }

  private async pullRemoteRow(
    row: ContextFiles,
    options: { skipLocalConflicts?: boolean } = {},
  ): Promise<void> {
    const relativePath = row.path;
    if (!isSafeSyncPath(relativePath)) {
      console.warn(`[sync] ignoring remote row with unsafe path: ${relativePath}`);
      return;
    }

    if (this.localDirtyPaths.has(relativePath) || this.ignoringPaths.has(relativePath)) {
      return;
    }

    const fileRef = asFileRef(row.file_ref);
    if (!fileRef?.sha256) {
      console.warn(`[sync] skipping ${relativePath}: missing file_ref.sha256`);
      return;
    }
    const remoteHash = fileRef.sha256;

    if (this.tombstones.shouldBlockRemotePull(relativePath, remoteHash)) {
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

    if (options.skipLocalConflicts && localHash !== null) {
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
    if (this.initialSyncTracker) {
      this.initialSyncTracker.downloaded += 1;
    }
  }

  private async removeLocalFile(relativePath: string): Promise<void> {
    if (!isSafeSyncPath(relativePath)) {
      return;
    }

    const contentHash = await readSyncFileHash(this.syncDir, relativePath);
    this.tombstones.mark(relativePath, contentHash);
    this.markIgnoring(relativePath);
    await removeSyncFile(this.syncDir, relativePath);
    await removeLocalRecord(this.localDb, relativePath);
    this.remotePaths.delete(relativePath);
    logFileDeleted(relativePath);
  }

  private async handleLiveRows(rows: ContextFiles[]): Promise<void> {
    const nextPaths = new Set(
      rows.map((row) => row.path).filter((path) => isSafeSyncPath(path)),
    );

    this.liveSnapshotsHandled += 1;
    const isInitialLiveSnapshot = this.liveSnapshotsHandled === 1;

    // The live client can emit an empty snapshot before initial rows arrive.
    // Never treat that as "delete everything we just downloaded".
    if (isInitialLiveSnapshot && nextPaths.size === 0 && this.remotePaths.size > 0) {
      return;
    }

    if (!isInitialLiveSnapshot) {
      for (const path of this.remotePaths) {
        if (!nextPaths.has(path)) {
          await this.removeLocalFile(path);
        }
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
      this.syncQueue.enqueue(() => this.handleLiveRows(rows));
    });
    console.log('[sync] live subscription active');
  }
}
