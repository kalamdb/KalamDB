import test from 'node:test';
import assert from 'node:assert/strict';
import { resolveKalamConnection } from '../src/db/client.js';
import { sha256Hex } from '../src/sync/file-store.js';

test('resolveKalamConnection uses kalam dev defaults', () => {
  assert.deepEqual(resolveKalamConnection({}), {
    url: 'http://127.0.0.1:2900',
    user: 'alice',
    password: 'alice123',
  });
});

test('sha256Hex hashes file bytes deterministically', () => {
  const hash = sha256Hex(new TextEncoder().encode('hello okf'));
  assert.match(hash, /^[a-f0-9]{64}$/);
  assert.equal(hash, sha256Hex(new TextEncoder().encode('hello okf')));
});
