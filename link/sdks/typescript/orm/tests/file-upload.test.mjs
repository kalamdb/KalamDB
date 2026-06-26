import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { kalamFile, rewriteSqlParamsForFileUploads } from '../dist/file-upload.js';

describe('rewriteSqlParamsForFileUploads', () => {
  it('replaces upload params with FILE placeholders and renumbers the rest', () => {
    const upload = kalamFile('upload', new File(['hello'], 'readme.md', { type: 'text/markdown' }));
    const rewritten = rewriteSqlParamsForFileUploads(
      'insert into demo.files (path, file_ref, updated_at) values ($1, $2, $3) on conflict (path) do update set file_ref = $4, updated_at = $5',
      ['notes/readme.md', upload, 100, upload, 101],
    );

    assert.equal(
      rewritten.sql,
      'insert into demo.files (path, file_ref, updated_at) values ($1, FILE("upload"), $2) on conflict (path) do update set file_ref = FILE("upload"), updated_at = $3',
    );
    assert.deepEqual(rewritten.params, ['notes/readme.md', 100, 101]);
    assert.equal(rewritten.files.upload, upload.blob);
  });

  it('leaves non-upload SQL unchanged', () => {
    const input = {
      sql: 'select path from demo.files where path = $1',
      params: ['notes/readme.md'],
    };
    const rewritten = rewriteSqlParamsForFileUploads(input.sql, input.params);
    assert.deepEqual(rewritten, { ...input, files: {} });
  });
});
