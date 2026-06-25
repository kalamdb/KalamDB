import { createHash } from 'node:crypto';
import type { KalamDBClient, RowData } from '@kalamdb/client';
import { TABLE } from './client.js';

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

export async function markDeleted(client: KalamDBClient, relativePath: string): Promise<void> {
  await client.query(
    `UPDATE ${TABLE}
     SET deleted = true, updated_at = NOW()
     WHERE path = $1`,
    [relativePath],
  );
}

export async function fetchServerSha256(
  client: KalamDBClient,
  relativePath: string,
): Promise<string | null> {
  const rows = await client.queryAll(
    `SELECT sha256, deleted FROM ${TABLE} WHERE path = $1`,
    [relativePath],
  );

  const row = rows[0];
  if (!row || row.deleted?.asBool()) {
    return null;
  }

  return row.sha256?.asString() ?? null;
}

export async function downloadFileText(
  client: KalamDBClient,
  relativePath: string,
  accessToken: string,
): Promise<string> {
  const rows = await client.queryRows(
    `SELECT path, file_ref FROM ${TABLE} WHERE path = $1 AND deleted = false`,
    TABLE,
    [relativePath],
  );

  const row = rows[0];
  if (!row) {
    throw new Error(`missing file row for ${relativePath}`);
  }

  const fileRef = row.file('file_ref');
  if (!fileRef) {
    throw new Error(`missing file_ref for ${relativePath}`);
  }

  const response = await fetch(fileRef.downloadUrl(), {
    headers: {
      Authorization: `Bearer ${accessToken}`,
    },
  });

  if (!response.ok) {
    throw new Error(`download failed (${response.status}) for ${relativePath}`);
  }

  return response.text();
}
