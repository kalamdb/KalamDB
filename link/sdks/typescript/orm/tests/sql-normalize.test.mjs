import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { sql } from 'drizzle-orm';
import { stripQuotedIdentifiers } from '../dist/query-normalize.js';
import { compileInlineQuery } from '../dist/sql.js';

describe('SQL normalization', () => {
  it('strips quoted identifiers without changing string literals', () => {
    const normalized = stripQuotedIdentifiers('SELECT "room" FROM "chat_demo"."messages" WHERE "content" = \'a"b\'');
    assert.equal(normalized, 'SELECT room FROM chat_demo.messages WHERE content = \'a"b\'');
  });

  it('preserves escaped single quotes and double quotes inside inline parameters', () => {
    const compiled = compileInlineQuery(sql`SELECT * FROM ${sql.raw('chat_demo.messages')} WHERE content = ${"a\"b and it's ok"}`);
    assert.equal(compiled.sql, "SELECT * FROM chat_demo.messages WHERE content = 'a\"b and it''s ok'");
    assert.deepEqual(compiled.params, []);
  });
});
