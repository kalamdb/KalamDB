import { createHash } from 'node:crypto';
import type { FileRef } from '@kalamdb/client';
import type { KalamDBClient } from '@kalamdb/client';
import { eq } from 'drizzle-orm';
import type { KalamDb } from '../db/client.js';
import { NAMESPACE, TABLE } from '../db/client.js';
import { context_files } from '../models/schema.generated.js';

const TABLE_NAME = 'context_files';

export function sha256Hex(bytes: Uint8Array): string {
  return createHash('sha256').update(bytes).digest('hex');
}

export function remoteContentHash(fileRef: FileRef | null | undefined): string | null {
  return fileRef?.sha256 ?? null;
}

export function guessMimeType(relativePath: string): string {
  const lower = relativePath.toLowerCase();
  if (lower.endsWith('.md')) {
    return 'text/markdown';
  }
  if (lower.endsWith('.json')) {
    return 'application/json';
  }
  if (lower.endsWith('.txt')) {
    return 'text/plain';
  }
  return 'application/octet-stream';
}

/**
 * Upload file bytes and write the metadata row. The backend computes
 * `file_ref.sha256` from the uploaded content.
 */
export async function upsertFile(
  client: KalamDBClient,
  input: {
    path: string;
    fileBytes: Uint8Array;
    mimeType: string;
  },
): Promise<void> {
  const blob = new Blob([Buffer.from(input.fileBytes)], { type: input.mimeType });
  const file = new File([blob], input.path.split('/').pop() ?? 'upload', { type: input.mimeType });
  const values = [input.path];

  const probe = await client.query(`SELECT path FROM ${TABLE} WHERE path = $1`, [input.path]);
  const exists = (probe.results?.[0]?.row_count ?? 0) > 0;

  if (exists) {
    await client.queryWithFiles(
      `UPDATE ${TABLE} SET
         file_ref = FILE("upload"),
         updated_at = NOW()
       WHERE path = $1`,
      { upload: file },
      values,
    );
    return;
  }

  await client.queryWithFiles(
    `INSERT INTO ${TABLE} (path, file_ref, updated_at)
     VALUES ($1, FILE("upload"), NOW())`,
    { upload: file },
    values,
  );
}

export async function fetchRemoteHash(db: KalamDb, relativePath: string): Promise<string | null> {
  const rows = await db
    .select({ file_ref: context_files.file_ref })
    .from(context_files)
    .where(eq(context_files.path, relativePath));

  return remoteContentHash(rows[0]?.file_ref);
}

/**
 * Download bytes for a specific `FileRef` and verify them against that same
 * ref's `sha256`. The caller must pass the exact `FileRef` it intends to write
 * (e.g. the one delivered by the live event) so the download URL and the hash
 * always describe the same file version. Re-querying the row here would let the
 * URL and the expected hash drift to different versions and produce spurious
 * "hash mismatch" errors during concurrent updates.
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

/** Look up a row's `file_ref` by path and download it. Used for ad-hoc reads/tests. */
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

  const fileRef = rows[0]?.file_ref;
  if (!fileRef) {
    throw new Error(`missing file row for ${relativePath}`);
  }

  return downloadFileBytes(baseUrl, fileRef, accessToken);
}
