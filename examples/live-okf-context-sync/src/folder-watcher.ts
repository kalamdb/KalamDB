/**
 * Watches the OKF sync folder and forwards file events to the sync worker.
 *
 * chokidar uses ignoreInitial: false so files touched while kalam dev was stopped
 * are picked up on the next run.
 */

import chokidar, { type FSWatcher } from 'chokidar';
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
};

type FolderEvent = 'add' | 'change' | 'unlink';

export async function watchSyncFolder(
  syncDir: string,
  handlers: FolderWatchHandlers,
  options: WatchSyncFolderOptions = {},
): Promise<FolderWatcher> {
  const { initialSync } = options;
  let initialScan = true;
  const queue = new TaskQueue();

  const handleEvent = (event: FolderEvent, absolutePath: string): void => {
    const rel = toRelativeSyncPath(syncDir, absolutePath);
    if (!isSafeSyncPath(rel)) {
      return;
    }

    if (event === 'add') {
      logFileAdded(rel);
      if (initialScan && initialSync) {
        initialSync.added += 1;
      }
      queue.enqueue(() => handlers.onUpsert(rel));
      return;
    }

    if (event === 'change') {
      logFileUpdated(rel);
      if (initialScan && initialSync) {
        initialSync.updated += 1;
      }
      queue.enqueue(() => handlers.onUpsert(rel));
      return;
    }

    logFileDeleted(rel);
    if (initialScan && initialSync) {
      initialSync.deleted += 1;
    }
    queue.enqueue(() => handlers.onDelete(rel));
  };

  const watcher: FSWatcher = chokidar.watch(syncDir, {
    ignoreInitial: false,
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
    watcher.once('error', reject);
  });

  await queue.waitIdle();
  initialScan = false;

  return {
    close: async () => {
      await watcher.close();
      await queue.waitIdle();
    },
  };
}
