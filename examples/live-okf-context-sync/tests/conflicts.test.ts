import test from 'node:test';
import assert from 'node:assert/strict';
import { conflictCopyPath, decideConflictAction } from '../src/conflicts.js';

test('decideConflictAction updates canonical path when server matches base', () => {
  const decision = decideConflictAction({
    relativePath: 'profile.md',
    localBaseSha256: 'aaa',
    serverSha256: 'aaa',
  });

  assert.deepEqual(decision, { kind: 'update-canonical', path: 'profile.md' });
});

test('decideConflictAction creates conflict copy when server diverged', () => {
  const at = new Date('2026-06-25T13:42:10.000Z');
  const decision = decideConflictAction({
    relativePath: 'profile.md',
    localBaseSha256: 'aaa',
    serverSha256: 'bbb',
  });

  assert.equal(decision.kind, 'create-conflict');
  if (decision.kind === 'create-conflict') {
    assert.equal(decision.canonicalPath, 'profile.md');
    assert.equal(conflictCopyPath('profile.md', at), 'profile.conflict-2026-06-25T13-42-10-000Z.md');
  }
});

test('conflictCopyPath preserves extension', () => {
  const at = new Date('2026-06-25T13:42:10.000Z');
  assert.equal(
    conflictCopyPath('projects/kalamdb.md', at),
    'projects/kalamdb.conflict-2026-06-25T13-42-10-000Z.md',
  );
});
