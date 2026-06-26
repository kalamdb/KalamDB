/**
 * Remote OKF file rows in KalamDB.
 *
 * Reads, writes, and deletes all go through Drizzle. FILE uploads pass a
 * `kalamFile()` value in `.values()` / `.set()`; `kalamDriver()` routes to
 * multipart SQL automatically.
 */

import type { FileRef } from '@kalamdb/client';
import { kalamFile } from '@kalamdb/orm';
import { eq } from 'drizzle-orm';
import type { KalamDb } from './db/client.js';
import { NAMESPACE } from './db/client.js';
import {
  buildUploadFile,
  guessMimeType,
  asFileRef,
  remoteContentHash,
  sha256Hex,
} from './lib/file-utils.js';
import { context_files } from './models/schema.generated.js';

const TABLE_NAME = 'context_files';

export { guessMimeType, remoteContentHash, sha256Hex };

/** Upsert path + bytes through Drizzle; the driver handles multipart upload. */
export async function upsertSyncFile(
  db: KalamDb,
  input: {
    path: string;
    fileBytes: Uint8Array;
    mimeType: string;
  },
): Promise<void> {
  const upload = buildUploadFile(input.path, input.fileBytes, input.mimeType);
  const now = new Date();
  const fileRef = kalamFile('upload', upload);

  await db
    .insert(context_files)
    .values({
      path: input.path,
      file_ref: fileRef,
      updated_at: now,
    })
    .onConflictDoUpdate({
      target: context_files.path,
      set: {
        file_ref: fileRef,
        updated_at: now,
      },
    });
}

export async function fetchRemoteHash(db: KalamDb, relativePath: string): Promise<string | null> {
  const rows = await db
    .select({ file_ref: context_files.file_ref })
    .from(context_files)
    .where(eq(context_files.path, relativePath));

  return remoteContentHash(rows[0]?.file_ref);
}

export async function deleteRemoteFile(db: KalamDb, relativePath: string): Promise<void> {
  await db.delete(context_files).where(eq(context_files.path, relativePath));
}

/**
 * Download bytes for a specific `FileRef` and verify them against that ref.
 * The caller must pass the exact ref from the live event so URL and hash match.
 */
export async function downloadFileBytes(
  baseUrl: string,
  fileRef: FileRef,
  accessToken: string,
): Promise<Uint8Array> {
  const response = await fetch(fileRef.getDownloadUrl(baseUrl, NAMESPACE, TABLE_NAME), {
    headers: {
      Authorization: `Bearer ${accessToken}`,
    },
  });

  if (!response.ok) {
    throw new Error(`download failed (${response.status}) for ${fileRef.name}`);
  }

  const bytes = new Uint8Array(await response.arrayBuffer());
  if (fileRef.sha256 && sha256Hex(bytes) !== fileRef.sha256) {
    throw new Error(`hash mismatch after download for ${fileRef.name}`);
  }

  return bytes;
}

/** Look up a row by path and download its `file_ref`. Used in tests. */
export async function downloadFileByPath(
  db: KalamDb,
  baseUrl: string,
  relativePath: string,
  accessToken: string,
): Promise<Uint8Array> {
  const rows = await db
    .select({ file_ref: context_files.file_ref })
    .from(context_files)
    .where(eq(context_files.path, relativePath));

  const fileRef = asFileRef(rows[0]?.file_ref);
  if (!fileRef) {
    throw new Error(`missing file row for ${relativePath}`);
  }

  return downloadFileBytes(baseUrl, fileRef, accessToken);
}
