import { describe, it, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { drizzle } from 'drizzle-orm/pg-proxy';
import { and, asc, desc, eq, gt, gte, inArray, like, lt, or } from 'drizzle-orm';
import { integer, text, timestamp } from 'drizzle-orm/pg-core';
import { kalamDriver, kTable } from '../dist/index.js';
import { requirePassword, createTestClient } from './helpers.mjs';

requirePassword();

let client;
let db;

const events = kTable.shared('test_filter.events', {
  id: text('id'),
  tenant: text('tenant'),
  category: text('category'),
  severity: integer('severity'),
  owner: text('owner'),
  created_at: timestamp('created_at', { mode: 'date' }),
});

const timeline = kTable.shared('test_filter.timeline', {
  id: text('id'),
  label: text('label'),
  created_at: timestamp('created_at', { mode: 'date' }),
});

before(async () => {
  client = createTestClient();
  await client.initialize();
  db = drizzle(kalamDriver(client));

  await client.query('CREATE NAMESPACE IF NOT EXISTS test_filter');
  await client.query('DROP TABLE IF EXISTS test_filter.events');
  await client.query('DROP TABLE IF EXISTS test_filter.timeline');
  await client.query(`
    CREATE TABLE test_filter.events (
      id TEXT PRIMARY KEY,
      tenant TEXT NOT NULL,
      category TEXT NOT NULL,
      severity INT NOT NULL,
      owner TEXT,
      created_at TIMESTAMP DEFAULT NOW()
    )
  `);
  await client.query(`
    CREATE TABLE test_filter.timeline (
      id TEXT PRIMARY KEY,
      label TEXT NOT NULL,
      created_at TIMESTAMP NOT NULL
    )
  `);

  await db.insert(events).values([
    { id: 'e1', tenant: 'acme', category: 'bug', severity: 3, owner: 'alice' },
    { id: 'e2', tenant: 'acme', category: 'ops', severity: 9, owner: 'bob' },
    { id: 'e3', tenant: 'beta', category: 'bug', severity: 10, owner: 'alice' },
    { id: 'e4', tenant: 'acme', category: 'info', severity: 1, owner: null },
    { id: 'e5', tenant: 'acme', category: 'bug', severity: 7, owner: 'carol' },
  ]);

  await db.insert(timeline).values([
    { id: 't1', label: 'morning', created_at: new Date('2026-04-30T09:15:00.000Z') },
    { id: 't2', label: 'noon', created_at: new Date('2026-04-30T12:00:00.000Z') },
    { id: 't3', label: 'evening', created_at: new Date('2026-04-30T18:45:00.000Z') },
  ]);
});

after(async () => {
  await client?.query('DROP TABLE IF EXISTS test_filter.events').catch(() => {});
  await client?.query('DROP TABLE IF EXISTS test_filter.timeline').catch(() => {});
  await client?.query('DROP SCHEMA IF EXISTS test_filter').catch(() => {});
  await client?.disconnect();
});

describe('kalamDriver filtering and temporal behavior', () => {
  it('supports compound filters with IN, comparison, ordering, and limit', async () => {
    const rows = await db
      .select({ id: events.id, severity: events.severity, category: events.category })
      .from(events)
      .where(and(
        eq(events.tenant, 'acme'),
        inArray(events.category, ['bug', 'ops']),
        gt(events.severity, 5),
      ))
      .orderBy(desc(events.severity), asc(events.id))
      .limit(2);

    assert.deepEqual(rows.map((row) => row.id), ['e2', 'e5']);
    assert.deepEqual(rows.map((row) => row.severity), [9, 7]);
  });

  it('supports OR predicates, LIKE, and nullable columns', async () => {
    const rows = await db
      .select({ id: events.id, owner: events.owner, category: events.category })
      .from(events)
      .where(and(
        eq(events.tenant, 'acme'),
        or(like(events.owner, 'ali%'), eq(events.category, 'info')),
      ))
      .orderBy(asc(events.id));

    assert.deepEqual(rows.map((row) => row.id), ['e1', 'e4']);
    assert.equal(rows[1].owner, null);
  });

  it('applies filtered UPDATE and DELETE statements through Drizzle builders', async () => {
    await db
      .update(events)
      .set({ owner: 'triaged' })
      .where(and(eq(events.tenant, 'acme'), eq(events.category, 'bug')));

    const updated = await db
      .select({ id: events.id, owner: events.owner })
      .from(events)
      .where(and(eq(events.tenant, 'acme'), eq(events.category, 'bug')))
      .orderBy(asc(events.id));

    assert.deepEqual(updated.map((row) => [row.id, row.owner]), [
      ['e1', 'triaged'],
      ['e5', 'triaged'],
    ]);

    await db.delete(events).where(and(eq(events.tenant, 'acme'), lt(events.severity, 2)));
    const remaining = await db.select().from(events).where(eq(events.id, 'e4'));
    assert.equal(remaining.length, 0);
  });

  it('round-trips Date parameters and filters timestamp ranges', async () => {
    const rows = await db
      .select({ id: timeline.id, label: timeline.label, created_at: timeline.created_at })
      .from(timeline)
      .where(and(
        gte(timeline.created_at, new Date('2026-04-30T12:00:00.000Z')),
        lt(timeline.created_at, new Date('2026-05-01T00:00:00.000Z')),
      ))
      .orderBy(asc(timeline.created_at));

    assert.deepEqual(rows.map((row) => row.id), ['t2', 't3']);
    for (const row of rows) {
      assert.ok(row.created_at instanceof Date);
      assert.ok(row.created_at.getTime() >= Date.parse('2026-04-30T12:00:00.000Z'));
    }
  });
});
