import test from 'node:test';
import assert from 'node:assert/strict';
import { FileRef } from '@kalamdb/client';
import {
  downloadFileBytes,
  guessMimeType,
  remoteContentHash,
  sha256Hex,
} from '../src/sync/file-store.js';

test('guessMimeType maps common extensions', () => {
  assert.equal(guessMimeType('readme.md'), 'text/markdown');
  assert.equal(guessMimeType('data.json'), 'application/json');
  assert.equal(guessMimeType('notes.txt'), 'text/plain');
  assert.equal(guessMimeType('blob.bin'), 'application/octet-stream');
});

test('remoteContentHash reads sha256 from FileRef', () => {
  const ref = new FileRef({
    id: '1',
    sub: 'f0001',
    name: 'index.md',
    size: 4,
    mime: 'text/markdown',
    sha256: 'abc123',
  });
  assert.equal(remoteContentHash(ref), 'abc123');
  assert.equal(remoteContentHash(null), null);
});

test('downloadFileBytes verifies content against FileRef.sha256', async () => {
  const content = '# hello\n';
  const bytes = new TextEncoder().encode(content);
  const hash = sha256Hex(bytes);
  const ref = new FileRef({
    id: '1',
    sub: 'f0001',
    name: 'index.md',
    size: bytes.byteLength,
    mime: 'text/markdown',
    sha256: hash,
  });

  const originalFetch = globalThis.fetch;
  globalThis.fetch = async () => new Response(bytes);

  try {
    const downloaded = await downloadFileBytes('http://127.0.0.1:2900', ref, 'token');
    assert.equal(new TextDecoder().decode(downloaded), content);
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test('downloadFileBytes rejects hash mismatch', async () => {
  const ref = new FileRef({
    id: '1',
    sub: 'f0001',
    name: 'index.md',
    size: 5,
    mime: 'text/markdown',
    sha256: 'deadbeef',
  });

  const originalFetch = globalThis.fetch;
  globalThis.fetch = async () => new Response('# other\n');

  try {
    await assert.rejects(
      () => downloadFileBytes('http://127.0.0.1:2900', ref, 'token'),
      /hash mismatch after download/,
    );
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test('downloadFileBytes surfaces HTTP failures', async () => {
  const ref = new FileRef({
    id: '1',
    sub: 'f0001',
    name: 'index.md',
    size: 0,
    mime: 'text/markdown',
    sha256: 'abc',
  });

  const originalFetch = globalThis.fetch;
  globalThis.fetch = async () => new Response('nope', { status: 404 });

  try {
    await assert.rejects(
      () => downloadFileBytes('http://127.0.0.1:2900', ref, 'token'),
      /download failed \(404\)/,
    );
  } finally {
    globalThis.fetch = originalFetch;
  }
});
