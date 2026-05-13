import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { FileRef } from '@kalamdb/client';
import initWasm from '@kalamdb/client/wasm';

await initWasm(
  {
    module_or_path: await readFile(
      new URL('../node_modules/@kalamdb/client/dist/wasm/kalam_client_bg.wasm', import.meta.url),
    ),
  },
);

// Test the file column's parse/serialize logic directly
// (same logic used in file-column.ts fromDriver/toDriver)

describe('FILE column type', () => {
  const sampleJson = JSON.stringify({
    id: '12345',
    sub: 'f0001',
    name: 'avatar.png',
    size: 4096,
    mime: 'image/png',
    sha256: 'abc123',
  });

  it('parses JSON string into FileRef', () => {
    const ref = FileRef.from(sampleJson);
    assert.ok(ref);
    assert.equal(ref.id, '12345');
    assert.equal(ref.name, 'avatar.png');
    assert.equal(ref.size, 4096);
    assert.equal(ref.mime, 'image/png');
  });

  it('returns null for null input', () => {
    const ref = FileRef.from(null);
    assert.equal(ref, null);
  });

  it('returns null for empty string', () => {
    const ref = FileRef.from('');
    assert.equal(ref, null);
  });

  it('generates download URL', () => {
    const ref = FileRef.from(sampleJson);
    assert.ok(ref);
    const url = ref.getDownloadUrl('http://localhost:2900', 'app', 'users');
    assert.equal(url, 'http://localhost:2900/v1/files/app/users/f0001/12345-avatar.png');
  });

  it('strips trailing slash from base URL', () => {
    const ref = FileRef.from(sampleJson);
    assert.ok(ref);
    const url = ref.getDownloadUrl('http://localhost:2900/', 'app', 'users');
    assert.equal(url, 'http://localhost:2900/v1/files/app/users/f0001/12345-avatar.png');
  });
});
