import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { stripDefaults } from '../dist/driver.js';

describe('stripDefaults', () => {
  it('removes DEFAULT column from single-default INSERT', () => {
    const input = 'INSERT INTO test.items (id, name) VALUES (DEFAULT, $1)';
    const result = stripDefaults(input, ['hello']);
    assert.equal(result.sql, 'INSERT INTO test.items (name) VALUES ($1)');
    assert.deepEqual(result.params, ['hello']);
  });

  it('removes multiple DEFAULT columns', () => {
    const input = 'INSERT INTO app.logs (id, message, created_at) VALUES (DEFAULT, $1, DEFAULT)';
    const result = stripDefaults(input, ['test message']);
    assert.equal(result.sql, 'INSERT INTO app.logs (message) VALUES ($1)');
    assert.deepEqual(result.params, ['test message']);
  });

  it('renumbers parameters after stripping defaults', () => {
    const input = 'INSERT INTO app.records (id, name, age, updated_at) VALUES (DEFAULT, $1, $2, DEFAULT)';
    const result = stripDefaults(input, ['alice', 30]);
    assert.equal(result.sql, 'INSERT INTO app.records (name, age) VALUES ($1, $2)');
    assert.deepEqual(result.params, ['alice', 30]);
  });

  it('passes through INSERT without DEFAULT unchanged', () => {
    const input = 'INSERT INTO test.items (name, age) VALUES ($1, $2)';
    const params = ['bob', 25];
    const result = stripDefaults(input, params);
    assert.equal(result.sql, input);
    assert.deepEqual(result.params, params);
  });

  it('passes through non-INSERT SQL unchanged', () => {
    const input = 'SELECT * FROM test.items WHERE id = $1';
    const params = [1];
    const result = stripDefaults(input, params);
    assert.equal(result.sql, input);
    assert.deepEqual(result.params, params);
  });

  it('passes through UPDATE SQL unchanged', () => {
    const input = 'UPDATE test.items SET name = $1 WHERE id = $2';
    const params = ['alice', 1];
    const result = stripDefaults(input, params);
    assert.equal(result.sql, input);
    assert.deepEqual(result.params, params);
  });

  it('handles case-insensitive DEFAULT', () => {
    const input = 'INSERT INTO test.items (id, name) VALUES (default, $1)';
    const result = stripDefaults(input, ['test']);
    assert.equal(result.sql, 'INSERT INTO test.items (name) VALUES ($1)');
    assert.deepEqual(result.params, ['test']);
  });

  it('handles DEFAULT as the only value', () => {
    const input = 'INSERT INTO test.counters (id) VALUES (DEFAULT)';
    const result = stripDefaults(input, []);
    assert.equal(result.sql, 'INSERT INTO test.counters () VALUES ()');
    assert.deepEqual(result.params, []);
  });

  it('preserves literal values mixed with params and DEFAULT', () => {
    const input = "INSERT INTO test.items (id, status, name) VALUES (DEFAULT, 'active', $1)";
    const result = stripDefaults(input, ['alice']);
    assert.equal(result.sql, "INSERT INTO test.items (status, name) VALUES ('active', $1)");
    assert.deepEqual(result.params, ['alice']);
  });

  it('correctly renumbers when DEFAULT is between two params', () => {
    const input = 'INSERT INTO test.items (a, b, c) VALUES ($1, DEFAULT, $2)';
    const result = stripDefaults(input, ['first', 'second']);
    assert.equal(result.sql, 'INSERT INTO test.items (a, c) VALUES ($1, $2)');
    assert.deepEqual(result.params, ['first', 'second']);
  });

  it('handles qualified table names with namespace', () => {
    const input = 'INSERT INTO my_namespace.my_table (id, val) VALUES (DEFAULT, $1)';
    const result = stripDefaults(input, [42]);
    assert.equal(result.sql, 'INSERT INTO my_namespace.my_table (val) VALUES ($1)');
    assert.deepEqual(result.params, [42]);
  });

  it('handles multi-row INSERT with DEFAULT', () => {
    const input = 'INSERT INTO test.items (id, name) VALUES (DEFAULT, $1), (DEFAULT, $2), (DEFAULT, $3)';
    const result = stripDefaults(input, ['one', 'two', 'three']);
    assert.equal(result.sql, 'INSERT INTO test.items (name) VALUES ($1), ($2), ($3)');
    assert.deepEqual(result.params, ['one', 'two', 'three']);
  });

  it('handles multi-row INSERT with multiple columns and DEFAULT', () => {
    const input = 'INSERT INTO test.items (id, name, age) VALUES (DEFAULT, $1, $2), (DEFAULT, $3, $4)';
    const result = stripDefaults(input, ['alice', 30, 'bob', 25]);
    assert.equal(result.sql, 'INSERT INTO test.items (name, age) VALUES ($1, $2), ($3, $4)');
    assert.deepEqual(result.params, ['alice', 30, 'bob', 25]);
  });

  it('handles multi-row INSERT without DEFAULT unchanged', () => {
    const input = 'INSERT INTO test.items (name, age) VALUES ($1, $2), ($3, $4)';
    const params = ['alice', 30, 'bob', 25];
    const result = stripDefaults(input, params);
    assert.equal(result.sql, input);
    assert.deepEqual(result.params, params);
  });

  it('leaves ON CONFLICT upserts unchanged', () => {
    const input =
      'insert into demo.context_files (path, file_ref) values ($1, FILE("upload")) on conflict (path) do update set file_ref = FILE("upload")';
    const params = ['notes/readme.md'];
    const result = stripDefaults(input, params);
    assert.equal(result.sql, input);
    assert.deepEqual(result.params, params);
  });
});

