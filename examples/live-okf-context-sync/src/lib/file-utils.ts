import { createHash } from 'node:crypto';
import { FileRef, parseFileRef } from '@kalamdb/client';

export function sha256Hex(bytes: Uint8Array): string {
  return createHash('sha256').update(bytes).digest('hex');
}

export function asFileRef(value: unknown): FileRef | null {
  return parseFileRef(value);
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

export function buildUploadFile(relativePath: string, fileBytes: Uint8Array, mimeType: string): File {
  const blob = new Blob([Buffer.from(fileBytes)], { type: mimeType });
  const name = relativePath.split('/').pop() ?? 'upload';
  return new File([blob], name, { type: mimeType });
}
