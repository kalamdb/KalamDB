import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { generateSchema } from '../dist/index.js';

const columnsJson = JSON.stringify([
  { column_name: 'id', ordinal_position: 1, data_type: 'Text', is_nullable: false, is_primary_key: true, default_value: 'None' },
  { column_name: 'body', ordinal_position: 2, data_type: 'Text', is_nullable: true, is_primary_key: false, default_value: 'None' },
  { column_name: '_seq', ordinal_position: 3, data_type: 'BigInt', is_nullable: false, is_primary_key: false, default_value: 'None' },
]);

const allTypesColumnsJson = JSON.stringify([
  { column_name: 'id', ordinal_position: 1, data_type: 'BigInt', is_nullable: false, is_primary_key: true, default_value: 'FunctionCall' },
  { column_name: 'flag', ordinal_position: 2, data_type: 'Boolean', is_nullable: false, is_primary_key: false, default_value: 'None' },
  { column_name: 'small_count', ordinal_position: 3, data_type: 'SmallInt', is_nullable: true, is_primary_key: false, default_value: 'None' },
  { column_name: 'count', ordinal_position: 4, data_type: 'Int', is_nullable: true, is_primary_key: false, default_value: 'None' },
  { column_name: 'score', ordinal_position: 5, data_type: 'Double', is_nullable: true, is_primary_key: false, default_value: 'None' },
  { column_name: 'ratio', ordinal_position: 6, data_type: 'Float', is_nullable: true, is_primary_key: false, default_value: 'None' },
  { column_name: 'payload', ordinal_position: 7, data_type: 'Json', is_nullable: true, is_primary_key: false, default_value: 'None' },
  { column_name: 'raw_bytes', ordinal_position: 8, data_type: 'Bytes', is_nullable: true, is_primary_key: false, default_value: 'None' },
  { column_name: 'doc_file', ordinal_position: 9, data_type: 'File', is_nullable: true, is_primary_key: false, default_value: 'None' },
  { column_name: 'vector', ordinal_position: 10, data_type: 'Embedding(384)', is_nullable: true, is_primary_key: false, default_value: 'None' },
  { column_name: 'public_id', ordinal_position: 11, data_type: 'Uuid', is_nullable: true, is_primary_key: false, default_value: 'None' },
  { column_name: 'amount', ordinal_position: 12, data_type: 'Decimal(10, 2)', is_nullable: true, is_primary_key: false, default_value: 'None' },
  { column_name: 'created_on', ordinal_position: 13, data_type: 'Date', is_nullable: true, is_primary_key: false, default_value: 'None' },
  { column_name: 'created_at', ordinal_position: 14, data_type: 'Timestamp', is_nullable: true, is_primary_key: false, default_value: 'None' },
  { column_name: 'updated_at', ordinal_position: 15, data_type: 'DateTime', is_nullable: true, is_primary_key: false, default_value: 'None' },
  { column_name: 'scheduled_for', ordinal_position: 16, data_type: 'Time', is_nullable: true, is_primary_key: false, default_value: 'None' },
  { column_name: 'select', ordinal_position: 17, data_type: 'Text', is_nullable: true, is_primary_key: false, default_value: 'None' },
]);

describe('generateSchema unit behavior', () => {
  it('emits Kalam table factories and keeps hidden columns opt-in', async () => {
    const client = {
      async query(sql) {
        assert.equal(sql, 'SHOW TABLES');
        return {
          results: [{
            named_rows: [
              {
                table_id: 'chat:messages',
                table_name: 'messages',
                namespace_id: 'chat',
                table_type: 'Shared',
                columns: columnsJson,
              },
              {
                table_id: 'chat:inbox',
                table_name: 'inbox',
                namespace_id: 'chat',
                table_type: 'Stream',
                columns: columnsJson,
              },
              {
                table_id: 'chat:all_types',
                table_name: 'all_types',
                namespace_id: 'chat',
                table_type: 'Shared',
                columns: allTypesColumnsJson,
              },
            ],
          }],
        };
      },
    };

    const schema = await generateSchema(client, { includeSystemColumns: true });

    assert.ok(schema.includes('kSystemColumns'));
    assert.ok(schema.includes('kTable'));
    assert.ok(schema.includes('export const chat_messages = kTable.shared("chat.messages"'));
    assert.ok(schema.includes('export const chat_messagesConfig = getKalamTableConfig(chat_messages)!;'));
    assert.ok(schema.includes('export const chat_inbox = kTable.stream("chat.inbox"'));
    assert.ok(schema.includes('...kSystemColumns(["_seq","_deleted"] as const),'));
    assert.ok(schema.includes('...kSystemColumns(["_seq"] as const),'));
    assert.ok(schema.includes('{ systemColumns: true }'));
    assert.ok(schema.includes('export type ChatMessages = typeof chat_messages.$inferSelect;'));
  });

  it('maps KalamDB datatypes to Drizzle column builders', async () => {
    const client = {
      async query(sql) {
        assert.equal(sql, 'SHOW TABLES');
        return {
          results: [{
            named_rows: [{
              table_id: 'chat:all_types',
              table_name: 'all_types',
              namespace_id: 'chat',
              table_type: 'Shared',
              columns: allTypesColumnsJson,
            }],
          }],
        };
      },
    };

    const schema = await generateSchema(client);

    assert.ok(schema.includes("import { bytes, embedding, file, getKalamTableConfig, kTable } from '@kalamdb/orm';"));
    assert.ok(schema.includes('boolean("flag")'));
    assert.ok(schema.includes('smallint("small_count")'));
    assert.ok(schema.includes('integer("count")'));
    assert.ok(schema.includes('text("id").default(sql``).primaryKey()'));
    assert.ok(schema.includes('doublePrecision("score")'));
    assert.ok(schema.includes('real("ratio")'));
    assert.ok(schema.includes('jsonb("payload")'));
    assert.ok(schema.includes('bytes("raw_bytes")'));
    assert.ok(schema.includes('file("doc_file")'));
    assert.ok(schema.includes('embedding("vector", 384)'));
    assert.ok(schema.includes('uuid("public_id")'));
    assert.ok(schema.includes('numeric("amount", { precision: 10, scale: 2 })'));
    assert.ok(schema.includes('date("created_on", { mode: \'date\' })'));
    assert.ok(schema.includes('timestamp("created_at", { mode: \'date\' })'));
    assert.ok(schema.includes('timestamp("updated_at", { mode: \'date\' })'));
    assert.ok(schema.includes('time("scheduled_for")'));
    assert.ok(schema.includes('select: text("select")'));
  });

  it('uses DESCRIBE fallback and renders complex real-world schemas with options', async () => {
    const queries = [];
    const client = {
      async query(sql) {
        queries.push(sql);
        if (sql === 'SHOW TABLES') {
          return {
            results: [{
              named_rows: [
                {
                  table_id: 'commerce:orders',
                  table_name: 'orders',
                  namespace_id: 'commerce',
                  table_type: 'User',
                  columns: JSON.stringify([
                    { column_name: 'id', data_type: 'BigInt', is_nullable: false },
                    { column_name: 'select', data_type: 'Text', is_nullable: false },
                  ]),
                },
                {
                  table_id: 'commerce:events',
                  table_name: 'events',
                  namespace_id: 'commerce',
                  table_type: 'Stream',
                  columns: JSON.stringify([
                    { column_name: 'event_id', ordinal_position: 1, data_type: 'Text', is_nullable: false, is_primary_key: true, default_value: 'None' },
                    { column_name: 'received_at', ordinal_position: 2, data_type: 'Timestamp(Microsecond)', is_nullable: false, is_primary_key: false, default_value: 'FunctionCall' },
                  ]),
                },
                {
                  table_id: 'archive:orders',
                  table_name: 'orders',
                  namespace_id: 'archive',
                  table_type: 'Shared',
                  columns: columnsJson,
                },
              ],
            }],
          };
        }

        if (sql === 'DESCRIBE commerce.orders') {
          return {
            results: [{
              named_rows: [
                { column_name: 'id', ordinal_position: 1, data_type: 'BigInt', is_nullable: false, is_primary_key: true, default_value: 'FunctionCall', column_comment: 'server generated id' },
                { column_name: 'tenant_id', ordinal_position: 2, data_type: 'Uuid', is_nullable: false, is_primary_key: false, default_value: 'None' },
                { column_name: 'status', ordinal_position: 3, data_type: 'Text', is_nullable: false, is_primary_key: false, default_value: "'pending'" },
                { column_name: 'total', ordinal_position: 4, data_type: { Decimal: { precision: 18, scale: 4 } }, is_nullable: false, is_primary_key: false, default_value: 'None' },
                { column_name: 'embedding', ordinal_position: 5, data_type: { Embedding: 8 }, is_nullable: true, is_primary_key: false, default_value: 'None' },
                { column_name: 'metadata', ordinal_position: 6, data_type: 'JSONB', is_nullable: true, is_primary_key: false, default_value: 'None' },
                { column_name: 'raw_blob', ordinal_position: 7, data_type: 'LargeBinary', is_nullable: true, is_primary_key: false, default_value: 'None' },
                { column_name: 'created_at', ordinal_position: 8, data_type: 'Timestamp(Nanosecond)', is_nullable: false, is_primary_key: false, default_value: 'FunctionCall' },
                { column_name: 'ship_date', ordinal_position: 9, data_type: 'Date32', is_nullable: true, is_primary_key: false, default_value: 'None' },
                { column_name: 'delivery_window', ordinal_position: 10, data_type: 'Time64(Microsecond)', is_nullable: true, is_primary_key: false, default_value: 'None' },
                { column_name: 'class', ordinal_position: 11, data_type: 'Text', is_nullable: true, is_primary_key: false, default_value: 'None' },
                { column_name: '_seq', ordinal_position: 12, data_type: 'BigInt', is_nullable: false, is_primary_key: false, default_value: 'None' },
              ],
            }],
          };
        }

        throw new Error(`unexpected query: ${sql}`);
      },
    };

    const schema = await generateSchema(client, {
      namespaces: ['commerce'],
      includeSystemColumns: 'all',
      bigIntMode: 'bigint',
    });

    assert.equal(queries[0], 'SHOW TABLES');
    assert.equal(queries.filter((query) => query === 'DESCRIBE commerce.orders').length, 1);
    assert.ok(schema.includes('export const orders = kTable.user("orders"'));
    assert.ok(schema.includes('export const events = kTable.stream("events"'));
    assert.ok(!schema.includes('archive_orders'));
    assert.ok(schema.includes('/** server generated id */'));
    assert.ok(schema.includes('id: bigint("id", { mode: "bigint" }).default(sql``).primaryKey()'));
    assert.ok(schema.includes('tenant_id: uuid("tenant_id").notNull()'));
    assert.ok(schema.includes('status: text("status").default(sql``).notNull()'));
    assert.ok(schema.includes('total: numeric("total", { precision: 18, scale: 4 }).notNull()'));
    assert.ok(schema.includes('embedding: embedding("embedding", 8)'));
    assert.ok(schema.includes('metadata: jsonb("metadata")'));
    assert.ok(schema.includes('raw_blob: bytes("raw_blob")'));
    assert.ok(schema.includes('created_at: timestamp("created_at", { mode: \'date\' }).default(sql``).notNull()'));
    assert.ok(schema.includes('ship_date: date("ship_date", { mode: \'date\' })'));
    assert.ok(schema.includes('delivery_window: time("delivery_window")'));
    assert.ok(schema.includes('class: text("class")'));
    assert.ok(schema.includes('...kSystemColumns(["_seq","_deleted","_commit_seq"] as const),'));
    assert.ok(schema.includes('}, { systemColumns: "all" });'));
    assert.ok(schema.includes('export const ordersConfig = getKalamTableConfig(orders)!;'));
    assert.ok(schema.includes('export type Orders = typeof orders.$inferSelect;'));
  });

  it('uses unqualified table names and simple exports for single-namespace output', async () => {
    const client = {
      async query(sql) {
        assert.equal(sql, 'SHOW TABLES');
        return {
          results: [{
            named_rows: [{
              table_id: 'chat:messages',
              table_name: 'messages',
              namespace_id: 'chat',
              table_type: 'Shared',
              columns: columnsJson,
            }],
          }],
        };
      },
    };

    const schema = await generateSchema(client, { namespaces: ['chat'] });

    assert.ok(schema.includes('export const messages = kTable.shared("messages"'));
    assert.ok(schema.includes('export const messagesConfig = getKalamTableConfig(messages)!;'));
    assert.ok(schema.includes('export type Messages = typeof messages.$inferSelect;'));
    assert.ok(!schema.includes('chat_messages'));
    assert.ok(!schema.includes('"chat.messages"'));
  });

  it('honors explicit system columns, bigint number mode, and disabled type aliases', async () => {
    const client = {
      async query(sql) {
        assert.equal(sql, 'SHOW TABLES');
        return {
          results: [{
            named_rows: [{
              table_id: 'metrics:samples',
              table_name: 'samples',
              namespace_id: 'metrics',
              table_type: 'Shared',
              columns: JSON.stringify([
                { column_name: 'id', ordinal_position: 1, data_type: 'BigInt', is_nullable: false, is_primary_key: true, default_value: 'None' },
                { column_name: 'value', ordinal_position: 2, data_type: 'Double', is_nullable: false, is_primary_key: false, default_value: 'None' },
              ]),
            }],
          }],
        };
      },
    };

    const schema = await generateSchema(client, {
      includeSystemColumns: ['_seq'],
      bigIntMode: 'number',
      includeTypeAliases: false,
    });

    assert.ok(schema.includes('...kSystemColumns(["_seq"] as const),'));
    assert.ok(schema.includes('}, { systemColumns: ["_seq"] });'));
    assert.ok(schema.includes('id: bigint("id", { mode: "number" }).primaryKey()'));
    assert.ok(schema.includes('value: doublePrecision("value").notNull()'));
    assert.ok(!schema.includes('export type MetricsSamples'));
    assert.ok(!schema.includes('export type NewMetricsSamples'));
  });
});
