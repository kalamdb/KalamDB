import { describe, it, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { drizzle } from 'drizzle-orm/pg-proxy';
import { bigint, text } from 'drizzle-orm/pg-core';
import { sql } from 'drizzle-orm';
import { kalamDriver, kTable } from '../dist/index.js';
import { requirePassword, createTestClient } from './helpers.mjs';

requirePassword();

const NS = 'test_orm_conflict_ret';

let client;
let db;

const users = kTable(`${NS}.users`, {
  id: bigint('id', { mode: 'number' }).primaryKey(),
  name: text('name').notNull(),
  age: bigint('age', { mode: 'number' }),
});

before(async () => {
  client = createTestClient();
  await client.initialize();
  db = drizzle(kalamDriver(client));

  await client.query(`CREATE NAMESPACE IF NOT EXISTS ${NS}`);
  await client.query(`DROP TABLE IF EXISTS ${NS}.users`);
  await client.query(`
    CREATE TABLE ${NS}.users (
      id BIGINT PRIMARY KEY,
      name TEXT NOT NULL,
      age BIGINT
    ) WITH (TYPE='SHARED', ACCESS_LEVEL='PUBLIC')
  `);
});

after(async () => {
  await client.query(`DROP TABLE IF EXISTS ${NS}.users`);
  await client.query(`DROP NAMESPACE IF EXISTS ${NS}`);
  await client?.disconnect();
});

describe('INSERT RETURNING via @kalamdb/orm', () => {
  it('returns inserted row with .returning()', async () => {
    const rows = await db
      .insert(users)
      .values({ id: 1, name: 'Nader', age: 3 })
      .returning();

    assert.equal(rows.length, 1);
    assert.equal(rows[0].id, 1);
    assert.equal(rows[0].name, 'Nader');
    assert.equal(rows[0].age, 3);
  });
});

describe('ON CONFLICT RETURNING via @kalamdb/orm', () => {
  it('ON CONFLICT DO UPDATE RETURNING returns updated row', async () => {
    const rows = await db
      .insert(users)
      .values({ id: 1, name: 'Nader Updated', age: 5 })
      .onConflictDoUpdate({
        target: users.id,
        set: { name: sql`excluded.name`, age: sql`excluded.age` },
      })
      .returning();

    assert.equal(rows.length, 1);
    assert.equal(rows[0].name, 'Nader Updated');
    assert.equal(rows[0].age, 5);
  });
});
