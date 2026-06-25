import { describe, it, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { eq } from 'drizzle-orm';
import { text, bigint, timestamp } from 'drizzle-orm/pg-core';
import { liveTable, file, kTable } from '../dist/index.js';
import { requirePassword, createTestClient } from './helpers.mjs';

requirePassword();

let client;

before(async () => {
  client = createTestClient();
  await client.initialize();
  await client.login();

  await client.query('CREATE NAMESPACE IF NOT EXISTS test_live');
  await client.query('DROP TABLE IF EXISTS test_live.items');
  await client.query('DROP TABLE IF EXISTS test_live.timeline');
  await client.query(`
    CREATE TABLE test_live.items (
      id BIGINT PRIMARY KEY DEFAULT SNOWFLAKE_ID(),
      name TEXT NOT NULL
    )
  `);
  await client.query(`
    CREATE TABLE test_live.timeline (
      id BIGINT PRIMARY KEY DEFAULT SNOWFLAKE_ID(),
      label TEXT NOT NULL,
      created_at TIMESTAMP DEFAULT NOW()
    )
  `);
  await client.query("INSERT INTO test_live.items (name) VALUES ('existing')");
  await client.query("INSERT INTO test_live.timeline (label) VALUES ('existing-timestamp')");
});

after(async () => {
  await client.query('DROP TABLE IF EXISTS test_live.items');
  await client.query('DROP TABLE IF EXISTS test_live.timeline');
  await client.query('DROP NAMESPACE IF EXISTS test_live');
  await client?.disconnect();
});

const test_live_items = kTable('test_live.items', {
  id: bigint('id', { mode: 'number' }),
  name: text('name'),
});

const test_live_timeline = kTable('test_live.timeline', {
  id: bigint('id', { mode: 'number' }),
  label: text('label'),
  created_at: timestamp('created_at', { mode: 'date' }),
});

async function waitForRows(client, table, predicate, timeoutMs = 10000, options = undefined) {
  let resolvePromise;
  let rejectPromise;
  const resultPromise = new Promise((resolve, reject) => {
    resolvePromise = resolve;
    rejectPromise = reject;
  });

  const timeout = setTimeout(() => rejectPromise(new Error('Timed out waiting for live rows')), timeoutMs);
  const unsub = await liveTable(client, table, (rows) => {
    if (predicate(rows)) {
      clearTimeout(timeout);
      resolvePromise([...rows]);
    }
  }, options);

  const result = await resultPromise;
  await unsub();
  await new Promise((r) => setTimeout(r, 100));
  return result;
}

describe('liveTable', () => {
  it('receives initial rows with correct types', async () => {
    const rows = await waitForRows(client, test_live_items, (r) => r.length > 0);

    assert.ok(Array.isArray(rows));
    assert.ok(rows.length >= 1);
    assert.equal(rows[0].name, 'existing');
    assert.ok(typeof rows[0].id === 'number');
  });

  it('receives realtime INSERT', async () => {
    const promise = waitForRows(client, test_live_items, (r) => r.some((row) => row.name === 'live-insert'));
    await new Promise((r) => setTimeout(r, 300));
    await client.query("INSERT INTO test_live.items (name) VALUES ('live-insert')");

    const rows = await promise;
    assert.ok(rows.some((r) => r.name === 'live-insert'));
  });

  it('receives realtime UPDATE', async () => {
    const promise = waitForRows(client, test_live_items, (r) => r.some((row) => row.name === 'updated-name'));
    await new Promise((r) => setTimeout(r, 300));
    await client.query("UPDATE test_live.items SET name = 'updated-name' WHERE name = 'live-insert'");

    const rows = await promise;
    assert.ok(rows.some((r) => r.name === 'updated-name'));
    assert.ok(!rows.some((r) => r.name === 'live-insert'));
  });

  it('receives realtime DELETE', async () => {
    const promise = waitForRows(client, test_live_items, (r) => !r.some((row) => row.name === 'updated-name'));
    await new Promise((r) => setTimeout(r, 300));
    await client.query("DELETE FROM test_live.items WHERE name = 'updated-name'");

    const rows = await promise;
    assert.ok(!rows.some((r) => r.name === 'updated-name'));
  });

  it('receives realtime DELETE for non-id primary keys', async () => {
    await client.query('DROP TABLE IF EXISTS test_live.paths');
    await client.query(`
      CREATE TABLE test_live.paths (
        path TEXT PRIMARY KEY,
        body TEXT NOT NULL
      )
    `);
    await client.query("INSERT INTO test_live.paths (path, body) VALUES ('docs/a.md', 'alpha')");

    const test_live_paths = kTable('test_live.paths', {
      path: text('path').primaryKey(),
      body: text('body'),
    });

    const promise = waitForRows(
      client,
      test_live_paths,
      (rows) => rows.length === 0,
    );
    await new Promise((r) => setTimeout(r, 300));
    await client.query("DELETE FROM test_live.paths WHERE path = 'docs/a.md'");

    const rows = await promise;
    assert.equal(rows.length, 0);

    await client.query('DROP TABLE IF EXISTS test_live.paths');
  });

  it('supports WHERE clauses without raw SQL strings', async () => {
    const rows = await waitForRows(
      client,
      test_live_items,
      (currentRows) => currentRows.length === 1 && currentRows[0].name === 'existing',
      10000,
      { where: eq(test_live_items.name, 'existing') },
    );

    assert.equal(rows.length, 1);
    assert.equal(rows[0].name, 'existing');
  });

  it('maps timestamp columns through Drizzle date mode for live rows', async () => {
    const rows = await waitForRows(
      client,
      test_live_timeline,
      (currentRows) => currentRows.some((row) => row.label === 'existing-timestamp'),
    );

    const row = rows.find((currentRow) => currentRow.label === 'existing-timestamp');
    assert.ok(row);
    assert.ok(row.created_at instanceof Date);
    assert.ok(!Number.isNaN(row.created_at.getTime()));
  });

  it('unsubscribe stops receiving updates', async () => {
    let callCount = 0;
    const unsub = await liveTable(client, test_live_items, () => {
      callCount++;
    });

    await new Promise((r) => setTimeout(r, 500));
    const countBefore = callCount;
    await unsub();

    await client.query("INSERT INTO test_live.items (name) VALUES ('after-unsub')");
    await new Promise((r) => setTimeout(r, 500));
    assert.equal(callCount, countBefore, 'should not receive more callbacks after unsubscribe');

    await client.query("DELETE FROM test_live.items WHERE name = 'after-unsub'");
  });

  it('maps typed rows with a WHERE clause', async () => {
    let resolvePromise;
    let rejectPromise;
    const resultPromise = new Promise((resolve, reject) => {
      resolvePromise = resolve;
      rejectPromise = reject;
    });

    const timeout = setTimeout(() => rejectPromise(new Error('Timed out waiting for typed live rows')), 10000);
    const unsub = await liveTable(
      client,
      test_live_items,
      (rows) => {
        if (!rows.some((row) => row.name === 'typed-live')) {
          return;
        }

        clearTimeout(timeout);
        resolvePromise(rows.find((row) => row.name === 'typed-live'));
      },
      { where: eq(test_live_items.name, 'typed-live') },
    );

    await client.query("INSERT INTO test_live.items (name) VALUES ('typed-live')");

    const row = await resultPromise;
    assert.equal(row.name, 'typed-live');
    assert.ok(typeof row.id === 'number');

    await unsub();
    await client.query("DELETE FROM test_live.items WHERE name = 'typed-live'");
  });

  it('handles FILE column type in live rows', async () => {
    await client.query('DROP TABLE IF EXISTS test_live.docs');
    await client.query(`
      CREATE TABLE test_live.docs (
        id BIGINT PRIMARY KEY DEFAULT SNOWFLAKE_ID(),
        title TEXT NOT NULL,
        attachment FILE
      )
    `);
    const fakeFile = JSON.stringify({
      id: '88888', sub: 'f0002', name: 'live-doc.pdf',
      size: 1024, mime: 'application/pdf', sha256: 'abc123',
    });
    await client.query(`INSERT INTO test_live.docs (title, attachment) VALUES ('live-test', '${fakeFile}')`);

    const test_live_docs = kTable('test_live.docs', {
      id: bigint('id', { mode: 'number' }),
      title: text('title'),
      attachment: file('attachment'),
    });

    const rows = await waitForRows(client, test_live_docs, (r) => r.length > 0);

    assert.equal(rows[0].title, 'live-test');
    assert.ok(rows[0].attachment);
    assert.equal(rows[0].attachment.name, 'live-doc.pdf');
    assert.equal(rows[0].attachment.size, 1024);

    await client.query('DROP TABLE IF EXISTS test_live.docs');
  });
});
