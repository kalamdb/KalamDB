import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { kalamFunctionDriver, toDrizzleRows } from '../dist/function-driver.js';
import { stripDefaults } from '../dist/strip-defaults.js';

describe('toDrizzleRows', () => {
  it('maps named objects to column arrays', () => {
    assert.deepEqual(toDrizzleRows([{ id: 'main', title: 'Main' }]), [['main', 'Main']]);
  });

  it('wraps a single object result', () => {
    assert.deepEqual(toDrizzleRows({ id: 1 }), [[1]]);
  });

  it('passes through array rows', () => {
    assert.deepEqual(toDrizzleRows([['a', 1]]), [['a', 1]]);
  });
});

describe('kalamFunctionDriver', () => {
  it('runs SELECT through query and INSERT through execute', async () => {
    const calls = [];
    const driver = kalamFunctionDriver({
      async query(sql, params) {
        calls.push({ kind: 'query', sql, params });
        return [{ id: 'main' }];
      },
      async execute(sql, params) {
        calls.push({ kind: 'execute', sql, params });
        return 1;
      },
    });

    const selected = await driver('select "id" from chat_demo.rooms where id = $1', ['main'], 'all');
    assert.deepEqual(selected.rows, [['main']]);

    const inserted = await driver(
      'insert into chat_demo.rooms ("id", "title") values ($1, $2)',
      ['lobby', 'Lobby'],
      'execute',
    );
    assert.deepEqual(inserted.rows, []);
    assert.equal(calls[0].kind, 'query');
    assert.match(calls[0].sql, /select id from chat_demo.rooms/i);
    assert.equal(calls[1].kind, 'execute');
  });

  it('wraps SQL with EXECUTE AS when requested', async () => {
    const calls = [];
    const driver = kalamFunctionDriver(
      {
        async query(sql, params) {
          calls.push({ sql, params });
          return [];
        },
        async execute() {
          return 0;
        },
      },
      { executeAs: 'alice' },
    );
    await driver('select 1', [], 'all');
    assert.match(calls[0].sql, /^EXECUTE AS 'alice' \(/);
  });

  it('rejects unsafe EXECUTE AS principals', async () => {
    const driver = kalamFunctionDriver(
      {
        async query() {
          return [];
        },
        async execute() {
          return 0;
        },
      },
      { executeAs: "alice'; DROP TABLE x; --" },
    );
    await assert.rejects(() => driver('select 1', [], 'all'), /unsupported user/);
  });
});

describe('stripDefaults', () => {
  it('keeps RETURNING when stripping DEFAULT columns', () => {
    const sql =
      'insert into chat_demo.messages (id, room, role, author, sender_username, content, reply_to, created_at) values (DEFAULT, $1, $2, $3, $4, $5, default, DEFAULT) returning id, room, role, author, sender_username, content, reply_to, created_at';
    const stripped = stripDefaults(sql, ['lobby', 'user', 'admin', 'admin', 'hello']);
    assert.equal(
      stripped.sql,
      'insert into chat_demo.messages (room, role, author, sender_username, content) VALUES ($1, $2, $3, $4, $5) returning id, room, role, author, sender_username, content, reply_to, created_at',
    );
    assert.deepEqual(stripped.params, ['lobby', 'user', 'admin', 'admin', 'hello']);
  });

  it('rewrites INSERT without RETURNING when only DEFAULT columns change', () => {
    const sql = 'insert into chat_demo.rooms (id, title, created_at) values ($1, $2, DEFAULT)';
    const stripped = stripDefaults(sql, ['lobby', 'Lobby']);
    assert.equal(stripped.sql, 'insert into chat_demo.rooms (id, title) VALUES ($1, $2)');
    assert.deepEqual(stripped.params, ['lobby', 'Lobby']);
  });
});

describe('bindFunctionOrm', () => {
  it('exposes drizzle and caches EXECUTE AS clients', async () => {
    const { bindFunctionOrm } = await import('../dist/function-driver.js');
    const orm = bindFunctionOrm({
      async query() {
        return [];
      },
      async execute() {
        return 0;
      },
    });
    assert.equal(typeof orm.select, 'function');
    assert.equal(typeof orm.insert, 'function');
    assert.equal(typeof orm.as, 'function');
    const alice = orm.as('alice');
    assert.equal(orm.as('alice'), alice);
    assert.notEqual(orm.as('bob'), alice);
  });
});
