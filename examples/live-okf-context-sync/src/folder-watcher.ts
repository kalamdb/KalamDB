/**
 * Watches the OKF sync folder and forwards file events to the sync worker.
 *
 * chokidar uses ignoreInitial: false so files touched while kalam dev was stopped
 * are picked up on the next run.
 */

import chokidar, { type FSWatcher } from 'chokidar';
import { existsSync } from 'node:fs';
import {
  isSafeSyncPath,
  shouldIgnoreWatchAbsolutePath,
  toRelativeSyncPath,
} from './lib/paths.js';
import { TaskQueue } from './helpers.js';
import {
  type InitialSyncTracker,
  logFileAdded,
  logFileDeleted,
  logFileUpdated,
} from './sync-log.js';

export type FolderWatchHandlers = {
  onUpsert: (relativePath: string) => Promise<void>;
  onDelete: (relativePath: string) => Promise<void>;
};

export type FolderWatcher = {
  close: () => Promise<void>;
};

export type WatchSyncFolderOptions = {
  initialSync?: InitialSyncTracker;
  /** Skip events caused by the sync worker writing or removing files itself. */
  shouldSuppressEvent?: (relativePath: string) => boolean;
  /** Share one queue with live pull so push/delete never races snapshot apply. */
  taskQueue?: TaskQueue;
  /** Called synchronously before enqueue so live pull cannot overwrite local edits. */
  onLocalChange?: (relativePath: string) => void;
  /** When true, startup files are pushed explicitly instead of via the initial scan. */
  ignoreInitial?: boolean;
};

type FolderEvent = 'add' | 'change' | 'unlink';

/** Wait for atomic editor saves (unlink + add/rename) before treating a path as deleted. */
const UNLINK_DEBOUNCE_MS = 400;

export async function watchSyncFolder(
  syncDir: string,
  handlers: FolderWatchHandlers,
  options: WatchSyncFolderOptions = {},
): Promise<FolderWatcher> {
  const { initialSync, shouldSuppressEvent, taskQueue, onLocalChange, ignoreInitial = false } = options;
  let initialScan = true;
  const queue = taskQueue ?? new TaskQueue();
  const pendingUnlinks = new Map<string, ReturnType<typeof setTimeout>>();

  const cancelPendingUnlink = (relativePath: string): void => {
    const timer = pendingUnlinks.get(relativePath);
    if (timer) {
      clearTimeout(timer);
      pendingUnlinks.delete(relativePath);
    }
  };

  const scheduleUnlink = (relativePath: string): void => {
    cancelPendingUnlink(relativePath);
    pendingUnlinks.set(
      relativePath,
      setTimeout(() => {
        pendingUnlinks.delete(relativePath);
        logFileDeleted(relativePath);
        if (initialScan && initialSync) {
          initialSync.deleted += 1;
        }
        queue.enqueue(() => handlers.onDelete(relativePath));
      }, UNLINK_DEBOUNCE_MS),
    );
  };

  const handleEvent = (event: FolderEvent, absolutePath: string): void => {
    const rel = toRelativeSyncPath(syncDir, absolutePath);
    if (!isSafeSyncPath(rel) || shouldSuppressEvent?.(rel)) {
      return;
    }

    if (event === 'unlink') {
      scheduleUnlink(rel);
      return;
    }

    cancelPendingUnlink(rel);

    if ((event === 'add' || event === 'change') && !existsSync(absolutePath)) {
      return;
    }

    if (event === 'add') {
      logFileAdded(rel);
      if (initialScan && initialSync) {
        initialSync.added += 1;
      }
      onLocalChange?.(rel);
      queue.enqueue(() => handlers.onUpsert(rel));
      return;
    }

    logFileUpdated(rel);
    if (initialScan && initialSync) {
      initialSync.updated += 1;
    }
    onLocalChange?.(rel);
    queue.enqueue(() => handlers.onUpsert(rel));
  };

  const watcher: FSWatcher = chokidar.watch(syncDir, {
    ignoreInitial,
    persistent: true,
    awaitWriteFinish: {
      stabilityThreshold: 250,
      pollInterval: 100,
    },
    ignored: (absolutePath) => shouldIgnoreWatchAbsolutePath(syncDir, absolutePath),
  });

  watcher
    .on('add', (absolutePath) => handleEvent('add', absolutePath))
    .on('change', (absolutePath) => handleEvent('change', absolutePath))
    .on('unlink', (absolutePath) => handleEvent('unlink', absolutePath));

  await new Promise<void>((resolve, reject) => {
    watcher.once('ready', () => resolve());
    watcher.on('error', (error: unknown) => {
      if ((error as NodeJS.ErrnoException).code === 'ENOENT') {
        return;
      }
      reject(error);
    });
  });

  await queue.waitIdle();
  initialScan = false;

  return {
    close: async () => {
      for (const timer of pendingUnlinks.values()) {
        clearTimeout(timer);
      }
      pendingUnlinks.clear();
      await watcher.close();
      await queue.waitIdle();
    },
  };
}
