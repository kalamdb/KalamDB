/** Sentinel for a tombstone created before content hash was known. */
export const TOMBSTONE_ANY = '*';

/**
 * Tracks paths deleted locally or via live pull so stale snapshots and other
 * clients cannot resurrect the same bytes during bulk deletes.
 */
export class SyncTombstones {
  private readonly values = new Map<string, string>();

  mark(path: string, hash: string | null): void {
    this.values.set(path, hash ?? TOMBSTONE_ANY);
  }

  clear(path: string): void {
    this.values.delete(path);
  }

  shouldBlockRemotePull(path: string, remoteHash: string): boolean {
    const tombstone = this.values.get(path);
    if (!tombstone) {
      return false;
    }
    if (tombstone === TOMBSTONE_ANY || tombstone === remoteHash) {
      return true;
    }
    this.values.delete(path);
    return false;
  }

  shouldBlockLocalPush(path: string, localHash: string): boolean {
    const tombstone = this.values.get(path);
    if (!tombstone) {
      return false;
    }
    if (tombstone === TOMBSTONE_ANY || tombstone === localHash) {
      return true;
    }
    this.values.delete(path);
    return false;
  }
}
