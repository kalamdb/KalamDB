import { createHash } from 'node:crypto';
import type { KalamDBClient } from '@kalamdb/client';
import { and, eq, sql } from 'drizzle-orm';
import type { KalamDb } from './client.js';
import { NAMESPACE, TABLE } from './client.js';
import { context_files } from './schema.generated.js';

const TABLE_NAME = 'context_files';

export type UploadedFile = {
  sha256: string;
  sizeBytes: number;
  mimeType: string;
};

export function sha256Hex(bytes: Uint8Array): string {
  return createHash('sha256').update(bytes).digest('hex');
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
 * Uploads file bytes and upserts the metadata row in one request. FILE columns
 * accept bytes through `queryWithFiles` + the `FILE("upload")` marker, so this
 * write stays on the raw client even though reads use the typed ORM.
 */
export async function upsertMetadata(
  client: KalamDBClient,
  input: {
    path: string;
    sha256: string;
    baseSha256: string | null;
    mimeType: string;
    sizeBytes: number;
    frontmatter: Record<string, unknown> | null;
    isConflict: boolean;
    canonicalPath: string | null;
    deleted: boolean;
    fileBytes: Uint8Array;
  },
): Promise<void> {
  const blob = new Blob([Buffer.from(input.fileBytes)], { type: input.mimeType });
  const file = new File([blob], input.path.split('/').pop() ?? 'upload', { type: input.mimeType });

  await client.queryWithFiles(
    `INSERT INTO ${TABLE} (
       path, file_ref, sha256, base_sha256, mime_type, size_bytes,
       frontmatter, is_conflict, canonical_path, deleted, updated_at
     ) VALUES (
       $1, FILE("upload"), $2, $3, $4, $5,
       $6, $7, $8, $9, NOW()
     )
     ON CONFLICT (path) DO UPDATE SET
       file_ref = FILE("upload"),
       sha256 = $2,
       base_sha256 = $3,
       mime_type = $4,
       size_bytes = $5,
       frontmatter = $6,
       is_conflict = $7,
       canonical_path = $8,
       deleted = $9,
       updated_at = NOW()`,
    { upload: file },
    [
      input.path,
      input.sha256,
      input.baseSha256,
      input.mimeType,
      input.sizeBytes,
      input.frontmatter ? JSON.stringify(input.frontmatter) : null,
      input.isConflict,
      input.canonicalPath,
      input.deleted,
    ],
  );
}

export async function markDeleted(db: KalamDb, relativePath: string): Promise<void> {
  await db
    .update(context_files)
    .set({ deleted: true, updated_at: sql`NOW()` })
    .where(eq(context_files.path, relativePath));
}

export async function fetchServerSha256(db: KalamDb, relativePath: string): Promise<string | null> {
  const rows = await db
    .select({ sha256: context_files.sha256, deleted: context_files.deleted })
    .from(context_files)
    .where(eq(context_files.path, relativePath));

  const row = rows[0];
  if (!row || row.deleted) {
    return null;
  }

  return row.sha256 ?? null;
}

export async function downloadFileText(
  db: KalamDb,
  baseUrl: string,
  relativePath: string,
  accessToken: string,
): Promise<string> {
  const rows = await db
    .select({ path: context_files.path, file_ref: context_files.file_ref })
    .from(context_files)
    .where(and(eq(context_files.path, relativePath), eq(context_files.deleted, false)));

  const row = rows[0];
  if (!row) {
    throw new Error(`missing file row for ${relativePath}`);
  }

  const fileRef = row.file_ref;
  if (!fileRef) {
    throw new Error(`missing file_ref for ${relativePath}`);
  }

  const response = await fetch(fileRef.getDownloadUrl(baseUrl, NAMESPACE, TABLE_NAME), {
    headers: {
      Authorization: `Bearer ${accessToken}`,
    },
  });

  if (!response.ok) {
    throw new Error(`download failed (${response.status}) for ${relativePath}`);
  }

  return response.text();
}
