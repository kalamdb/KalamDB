import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { drizzle } from 'drizzle-orm/pg-proxy';
import { bigint, text } from 'drizzle-orm/pg-core';
import { sql } from 'drizzle-orm';
import { compileQuery, kTable, kalamDriver, queryWithFiles } from '../dist/index.js';

const files = kTable('demo.context_files', {
  path: text('path').primaryKey(),
  file_ref: text('file_ref').notNull(),
  updated_at: bigint('updated_at', { mode: 'number' }),
});

describe('compileQuery', () => {
  it('normalizes quoted identifiers from Drizzle builders', () => {
    const compiled = compileQuery(
      sql`SELECT ${files.path} FROM ${files} WHERE ${files.path} = ${'notes/readme.md'}`,
    );

    assert.match(compiled.sql, /SELECT demo\.context_files\.path FROM demo\.context_files WHERE demo\.context_files\.path = \$1/);
    assert.deepEqual(compiled.params, ['notes/readme.md']);
  });
});

describe('queryWithFiles', () => {
  it('compiles ORM upserts before calling client.queryWithFiles', async () => {
    const calls = [];
    const client = {
      async query() {
        return { status: 'success', results: [] };
      },
      async queryWithFiles(sqlText, uploadFiles, params, onProgress) {
        calls.push({ sqlText, uploadFiles, params, onProgress });
        return { status: 'success', results: [] };
      },
    };
    const db = drizzle(kalamDriver(client));

    const upload = new File(['hello'], 'readme.md', { type: 'text/markdown' });
    const now = new Date('2026-06-26T12:00:00.000Z');

    await queryWithFiles(
      client,
      db
        .insert(files)
        .values({
          path: 'notes/readme.md',
          file_ref: sql.raw('FILE("upload")'),
          updated_at: now,
        })
        .onConflictDoUpdate({
          target: files.path,
          set: {
            file_ref: sql.raw('FILE("upload")'),
            updated_at: now,
          },
        }),
      { upload },
    );

    assert.equal(calls.length, 1);
    assert.match(calls[0].sqlText, /insert into demo\.context_files/i);
    assert.match(calls[0].sqlText, /on conflict/i);
    assert.match(calls[0].sqlText, /FILE\("upload"\)/);
    assert.ok(Array.isArray(calls[0].params));
    assert.equal(calls[0].uploadFiles.upload, upload);
  });

  it('passes raw SQL through unchanged', async () => {
    const calls = [];
    const client = {
      async queryWithFiles(sqlText, uploadFiles, params) {
        calls.push({ sqlText, uploadFiles, params });
        return { status: 'success', results: [] };
      },
    };

    const upload = new File(['x'], 'x.txt', { type: 'text/plain' });
    await queryWithFiles(
      client,
      'INSERT INTO demo.context_files (path, file_ref) VALUES ($1, FILE("upload"))',
      { upload },
      ['notes/x.txt'],
    );

    assert.equal(calls[0].sqlText, 'INSERT INTO demo.context_files (path, file_ref) VALUES ($1, FILE("upload"))');
    assert.deepEqual(calls[0].params, ['notes/x.txt']);
  });
});
