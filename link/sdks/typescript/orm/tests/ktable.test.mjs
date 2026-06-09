import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { sql } from 'drizzle-orm';
import { PgDialect, text } from 'drizzle-orm/pg-core';
import { getTableColumns } from 'drizzle-orm';
import {
  configureKalamOrm,
  getKalamTableConfig,
  kSystemColumns,
  kTable,
} from '../dist/index.js';

describe('kTable', () => {
  it('preserves Drizzle table columns while attaching Kalam metadata', () => {
    configureKalamOrm({});
    const orders = kTable.shared('app.orders', {
      id: text('id').notNull(),
      status: text('status'),
    });

    const columns = getTableColumns(orders);
    const config = getKalamTableConfig(orders);

    assert.ok(columns.id);
    assert.ok(columns.status);
    assert.equal(config.tableType, 'shared');
    assert.equal(config.qualifiedName, 'app.orders');
    assert.equal(config.namespace, 'app');
    assert.equal(config.name, 'orders');
    assert.equal(config.tableId, 'app:orders');
    assert.deepEqual(config.systemColumns, []);
  });

  it('applies the configured default namespace to unqualified table names', () => {
    configureKalamOrm({ namespace: 'app' });
    const orders = kTable.shared('orders', {
      id: text('id').notNull(),
    });
    const config = getKalamTableConfig(orders);
    const compiled = new PgDialect().sqlToQuery(sql`select * from ${orders}`.inlineParams());

    assert.equal(config.qualifiedName, 'app.orders');
    assert.equal(config.namespace, 'app');
    assert.equal(config.name, 'orders');
    assert.equal(config.tableId, 'app:orders');
    assert.equal(compiled.sql, 'select * from "app"."orders"');

    configureKalamOrm({});
  });

  it('adds public MVCC system columns when requested', () => {
    configureKalamOrm({});
    const messages = kTable.user('chat.messages', {
      id: text('id').notNull(),
      body: text('body'),
    }, { systemColumns: true });

    const columns = getTableColumns(messages);
    const config = getKalamTableConfig(messages);

    assert.ok(columns._seq);
    assert.ok(columns._deleted);
    assert.equal(columns._seq.name, '_seq');
    assert.equal(columns._deleted.name, '_deleted');
    assert.deepEqual(config.systemColumns, ['_seq', '_deleted']);
  });

  it('uses stream-safe defaults for system columns', () => {
    configureKalamOrm({});
    const events = kTable.stream('app.events', {
      id: text('id').notNull(),
    }, { systemColumns: true });

    const columns = getTableColumns(events);
    const config = getKalamTableConfig(events);

    assert.ok(columns._seq);
    assert.equal(columns._deleted, undefined);
    assert.deepEqual(config.systemColumns, ['_seq']);
  });

  it('keeps explicit system-column helper available for Drizzle-first schemas', () => {
    configureKalamOrm({});
    const audit = kTable('app.audit', {
      ...kSystemColumns(),
      id: text('id').notNull(),
    }, { tableType: 'shared' });

    const columns = getTableColumns(audit);
    const config = getKalamTableConfig(audit);

    assert.ok(columns._seq);
    assert.ok(columns._deleted);
    assert.equal(config.tableType, 'shared');
    assert.deepEqual(config.systemColumns, []);
  });

  it('preserves Drizzle extra config as the third argument', () => {
    configureKalamOrm({});
    const table = kTable.shared('app.indexed', {
      id: text('id').notNull(),
    }, () => [], { systemColumns: true });

    const columns = getTableColumns(table);
    assert.ok(columns.id);
    assert.ok(columns._seq);
  });
});
