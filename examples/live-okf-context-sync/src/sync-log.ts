/**
 * Console logging helpers for the OKF folder sync example.
 */

/** Human-readable byte size for push/download log lines. */
export function formatSize(bytes: number): string {
  if (bytes < 1024) {
    return `${bytes} B`;
  }
  if (bytes < 1024 * 1024) {
    return `${(bytes / 1024).toFixed(1)} KB`;
  }
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/** Counts folder and sync actions during the startup scan. */
export class InitialSyncTracker {
  added = 0;
  updated = 0;
  deleted = 0;
  pushed = 0;
  downloaded = 0;

  totalChanges(): number {
    return this.added + this.updated + this.deleted;
  }
}

export function logFileAdded(relativePath: string): void {
  console.log(`[sync] file '${relativePath}' added`);
}

export function logFileUpdated(relativePath: string): void {
  console.log(`[sync] file '${relativePath}' updated`);
}

export function logFileDeleted(relativePath: string): void {
  console.log(`[sync] file '${relativePath}' deleted`);
}

export type FilePushLog = {
  path: string;
  seq?: string | null;
  sizeBytes: number;
};

export function logFilePushed(event: FilePushLog): void {
  const seq = event.seq ? ` _seq=${event.seq}` : '';
  console.log(`[sync] pushed path='${event.path}'${seq} size=${formatSize(event.sizeBytes)}`);
}

export function logFileDownloaded(relativePath: string, sizeBytes: number): void {
  console.log(`[sync] file '${relativePath}' downloaded from server (${formatSize(sizeBytes)})`);
}

export function logInitialSyncStarted(folder: string): void {
  console.log(`[sync] initial sync for folder '${folder}' started ...`);
}

export function logInitialSyncCompleted(folder: string, tracker: InitialSyncTracker): void {
  const total = tracker.totalChanges();
  console.log(
    `[sync] initial sync for folder '${folder}' completed: `
    + `${tracker.added} added, ${tracker.updated} updated, ${tracker.deleted} deleted, `
    + `${tracker.pushed} pushed, ${tracker.downloaded} downloaded `
    + `(${total} total changes)`,
  );
}
