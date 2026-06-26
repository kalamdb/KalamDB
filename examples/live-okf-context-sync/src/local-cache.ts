/**
 * SQLite metadata cache for the OKF folder sync example.
 */

import { eq } from 'drizzle-orm';
import type { LocalDb } from './db/local-db.js';
import { local_context_files, pending_uploads } from './models/schema.local.js';

export type LocalSyncState = {
  cachedHash: string | null;
  hasPending: boolean;
};

export async function readLocalSyncState(db: LocalDb, relativePath: string): Promise<LocalSyncState> {
  const [localRow, pendingRow] = await Promise.all([
    db
      .select({ sha256: local_context_files.sha256 })
      .from(local_context_files)
      .where(eq(local_context_files.path, relativePath)),
    db
      .select({ path: pending_uploads.path })
      .from(pending_uploads)
      .where(eq(pending_uploads.path, relativePath)),
  ]);

  return {
    cachedHash: localRow[0]?.sha256 ?? null,
    hasPending: pendingRow.length > 0,
  };
}

export async function hasLocalRecord(db: LocalDb, relativePath: string): Promise<boolean> {
  const rows = await db
    .select({ path: local_context_files.path })
    .from(local_context_files)
    .where(eq(local_context_files.path, relativePath));
  return rows.length > 0;
}

export async function listTrackedPaths(db: LocalDb): Promise<string[]> {
  const rows = await db.select({ path: local_context_files.path }).from(local_context_files);
  return rows.map((row) => row.path);
}

export async function recordSyncedFile(
  db: LocalDb,
  relativePath: string,
  sha256: string,
  at = new Date(),
): Promise<void> {
  await db
    .insert(local_context_files)
    .values({ path: relativePath, sha256, created_at: at, updated_at: at })
    .onConflictDoUpdate({
      target: local_context_files.path,
      set: { sha256, updated_at: at },
    });
}

export async function queuePendingUpload(
  db: LocalDb,
  relativePath: string,
  sha256: string,
  error: unknown,
): Promise<void> {
  const now = new Date();
  await db
    .insert(pending_uploads)
    .values({ path: relativePath, sha256, updated_at: now, last_error: String(error) })
    .onConflictDoUpdate({
      target: pending_uploads.path,
      set: { sha256, updated_at: now, last_error: String(error) },
    });
  console.warn(`[sync] queued pending upload for ${relativePath}`);
}

export async function clearPendingUpload(db: LocalDb, relativePath: string): Promise<void> {
  await db.delete(pending_uploads).where(eq(pending_uploads.path, relativePath));
}

export async function removeLocalRecord(db: LocalDb, relativePath: string): Promise<void> {
  await db.delete(local_context_files).where(eq(local_context_files.path, relativePath));
}

export async function listPendingUploads(db: LocalDb): Promise<Array<{ path: string }>> {
  return db.select({ path: pending_uploads.path }).from(pending_uploads);
}
