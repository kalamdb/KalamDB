/**
 * Apollo-inspired real-world app flow tests for KalamDB client.
 *
 * These are intentionally higher-level than the existing auth/query/reconnect
 * slices. They mimic browser/app patterns that Apollo integration and client
 * tests exercise: initial query plus live follow-up, remount/filter changes,
 * multiple observers, teardown on final unsubscribe, lazy boot, and mixed
 * auth/query/live sessions.
 *
 * Run: node --test tests/e2e/realworld/apollo-inspired.test.mjs
 */

import { test, describe, before, after } from 'node:test';
import assert from 'node:assert/strict';
import {
  SERVER_URL,
  ADMIN_USER,
  ADMIN_PASS,
  connectJwtClient,
  ensureNamespace,
  dropTable,
  jwtAuthProvider,
  sleep,
  uniqueName,
} from '../helpers.mjs';
import {
  Auth,
  createClient,
  createRawSqlLiveDescriptor,
} from '../../../dist/src/index.js';

async function waitFor(predicate, timeoutMs = 15_000, intervalMs = 50) {
  const started = Date.now();
  while (!predicate()) {
    if (Date.now() - started > timeoutMs) {
      throw new Error(`Timed out waiting for condition after ${timeoutMs}ms`);
    }
    await sleep(intervalMs);
  }
}

async function shutdownClient(client) {
  if (!client) {
    return;
  }

  if (typeof client.shutdown === 'function') {
    await client.shutdown();
    return;
  }

  await client.disconnect();
}

function mapFeedRow(row) {
  return {
    id: row.id.asInt(),
    room: row.room.asString(),
    body: row.body.asString(),
    severity: row.severity.asString(),
    created_at: row.created_at.asDate().toISOString(),
    createdAt: row.created_at.asDate().toISOString(),
  };
}

function mapIssueRow(row) {
  return {
    id: row.id.asInt(),
    title: row.title.asString(),
    status: row.status.asString(),
    score: row.score.asFloat(),
  };
}

function latestRows(snapshots) {
  return snapshots.at(-1) ?? [];
}

function latestIds(snapshots) {
  return latestRows(snapshots).map((row) => row.id);
}

function sortRowsById(rows) {
  return [...rows].sort((left, right) => left.id - right.id);
}

function snapshotHasId(snapshots, id) {
  return snapshots.some((rows) => rows.some((row) => row.id === id));
}

function controllerSnapshotRows(snapshots) {
  return snapshots.at(-1)?.rows ?? [];
}

async function insertFeedRow(writer, table, { id, room, body, severity, createdAt }) {
  await writer.query(
    `INSERT INTO ${table} (id, room, body, severity, created_at)
     VALUES (${id}, '${room}', '${body}', '${severity}', '${createdAt}')`,
  );
}

describe('Apollo-inspired real-world app flows', { timeout: 180_000 }, () => {
  let client;
  const ns = uniqueName('ts_apollo_like');

  before(async () => {
    client = await connectJwtClient();
    await ensureNamespace(client, ns);
  });

  after(async () => {
    try {
      await client.unsubscribeAll();
    } catch (_) {
      // Ignore cleanup drift from failing tests.
    }
    await shutdownClient(client);
  });

  test('dashboard feed controller keeps the newest three rows', async () => {
    const table = `${ns}.${uniqueName('feed_ctrl')}`;
    const writer = await connectJwtClient();

    await client.query(
      `CREATE TABLE IF NOT EXISTS ${table} (
        id INT PRIMARY KEY,
        room TEXT NOT NULL,
        body TEXT NOT NULL,
        severity TEXT NOT NULL,
        created_at TIMESTAMP
      )`,
    );

    const descriptor = createRawSqlLiveDescriptor(
      `SELECT id, room, body, severity, created_at FROM ${table} WHERE room = 'main' ORDER BY created_at DESC LIMIT 3`,
      { mapRow: mapFeedRow, getKey: (row) => row.id },
    );
    const controller = client.createLiveQueryController(descriptor, { lastRows: 0 });
    const snapshots = [];
    controller.subscribe((snapshot) => snapshots.push(snapshot));

    try {
      await controller.start();
      await waitFor(() => snapshots.some((snapshot) => snapshot.status === 'live'));

      for (const row of [
        { id: 61001, room: 'main', body: 'first', severity: 'info', createdAt: '2026-04-10T10:00:00Z' },
        { id: 61002, room: 'main', body: 'second', severity: 'info', createdAt: '2026-04-10T10:01:00Z' },
        { id: 61003, room: 'main', body: 'third', severity: 'warn', createdAt: '2026-04-10T10:02:00Z' },
        { id: 61004, room: 'main', body: 'fourth', severity: 'error', createdAt: '2026-04-10T10:03:00Z' },
      ]) {
        await insertFeedRow(writer, table, row);
      }

      await waitFor(() => controllerSnapshotRows(snapshots).length === 3);
      assert.deepEqual(
        controllerSnapshotRows(snapshots).map((row) => row.id),
        [61004, 61003, 61002],
      );
    } finally {
      await controller.dispose();
      await dropTable(client, table);
      await shutdownClient(writer);
    }
  });

  test('changing the dashboard filter tears down the old live query and mounts the new one', async () => {
    const table = `${ns}.${uniqueName('filter_swap')}`;
    const writer = await connectJwtClient();

    await client.query(
      `CREATE TABLE IF NOT EXISTS ${table} (
        id INT PRIMARY KEY,
        room TEXT NOT NULL,
        body TEXT NOT NULL,
        severity TEXT NOT NULL,
        created_at TIMESTAMP
      )`,
    );

    const mainDescriptor = createRawSqlLiveDescriptor(
      `SELECT id, room, body, severity, created_at FROM ${table} WHERE room = 'main' ORDER BY created_at DESC LIMIT 5`,
      { mapRow: mapFeedRow, getKey: (row) => row.id },
    );
    const billingDescriptor = createRawSqlLiveDescriptor(
      `SELECT id, room, body, severity, created_at FROM ${table} WHERE room = 'billing' ORDER BY created_at DESC LIMIT 5`,
      { mapRow: mapFeedRow, getKey: (row) => row.id },
    );

    const mainController = client.createLiveQueryController(mainDescriptor, { lastRows: 0 });
    const billingController = client.createLiveQueryController(billingDescriptor, { lastRows: 0 });
    const mainSnapshots = [];
    const billingSnapshots = [];
    mainController.subscribe((snapshot) => mainSnapshots.push(snapshot));
    billingController.subscribe((snapshot) => billingSnapshots.push(snapshot));

    try {
      await mainController.start();
      await waitFor(() => mainSnapshots.some((snapshot) => snapshot.status === 'live'));

      await insertFeedRow(writer, table, {
        id: 62001,
        room: 'main',
        body: 'main-first',
        severity: 'info',
        createdAt: '2026-04-10T11:00:00Z',
      });
      await insertFeedRow(writer, table, {
        id: 62002,
        room: 'billing',
        body: 'billing-hidden',
        severity: 'warn',
        createdAt: '2026-04-10T11:01:00Z',
      });

      await waitFor(() => controllerSnapshotRows(mainSnapshots).some((row) => row.id === 62001));
      assert.deepEqual(controllerSnapshotRows(mainSnapshots).map((row) => row.id), [62001]);

      await mainController.dispose();
      await billingController.start();
      await waitFor(() => billingSnapshots.some((snapshot) => snapshot.status === 'live'));

      const mainLengthBefore = controllerSnapshotRows(mainSnapshots).length;
      await insertFeedRow(writer, table, {
        id: 62003,
        room: 'main',
        body: 'main-after-switch',
        severity: 'error',
        createdAt: '2026-04-10T11:02:00Z',
      });
      await insertFeedRow(writer, table, {
        id: 62004,
        room: 'billing',
        body: 'billing-after-switch',
        severity: 'info',
        createdAt: '2026-04-10T11:03:00Z',
      });

      await waitFor(() => controllerSnapshotRows(billingSnapshots).some((row) => row.id === 62004));
      assert.equal(controllerSnapshotRows(mainSnapshots).length, mainLengthBefore);
      assert.deepEqual(controllerSnapshotRows(billingSnapshots).map((row) => row.id), [62004]);
    } finally {
      await billingController.dispose();
      await dropTable(client, table);
      await shutdownClient(writer);
    }
  });

  test('two widgets on the same SQL both receive updates', async () => {
    const table = `${ns}.${uniqueName('widgets_same_sql')}`;
    const writer = await connectJwtClient();

    await client.query(
      `CREATE TABLE IF NOT EXISTS ${table} (
        id INT PRIMARY KEY,
        title TEXT NOT NULL,
        status TEXT NOT NULL,
        score DOUBLE
      )`,
    );

    const widgetA = [];
    const widgetB = [];
    const sql = `SELECT id, title, status, score FROM ${table}`;

    try {
      const stopA = await client.live(sql, (rows) => widgetA.push(sortRowsById(rows.map(mapIssueRow))), { lastRows: 0 });
      const stopB = await client.live(sql, (rows) => widgetB.push(sortRowsById(rows.map(mapIssueRow))), { lastRows: 0 });

      await writer.query(`INSERT INTO ${table} (id, title, status, score) VALUES (63001, 'alpha', 'open', 1.5)`);

      await waitFor(() => snapshotHasId(widgetA, 63001) && snapshotHasId(widgetB, 63001));
      assert.deepEqual(latestIds(widgetA), [63001]);
      assert.deepEqual(latestIds(widgetB), [63001]);

      await stopB();
      await stopA();
    } finally {
      await dropTable(client, table);
      await shutdownClient(writer);
    }
  });

  test('unsubscribing one widget leaves the sibling widget live', async () => {
    const table = `${ns}.${uniqueName('widgets_sibling')}`;
    const writer = await connectJwtClient();

    await client.query(
      `CREATE TABLE IF NOT EXISTS ${table} (
        id INT PRIMARY KEY,
        title TEXT NOT NULL,
        status TEXT NOT NULL,
        score DOUBLE
      )`,
    );

    const widgetA = [];
    const widgetB = [];
    const sql = `SELECT id, title, status, score FROM ${table}`;

    try {
      const stopA = await client.live(sql, (rows) => widgetA.push(sortRowsById(rows.map(mapIssueRow))), { lastRows: 0 });
      const stopB = await client.live(sql, (rows) => widgetB.push(sortRowsById(rows.map(mapIssueRow))), { lastRows: 0 });

      await writer.query(`INSERT INTO ${table} (id, title, status, score) VALUES (64001, 'first', 'open', 1.0)`);
      await waitFor(() => snapshotHasId(widgetA, 64001) && snapshotHasId(widgetB, 64001));

      const widgetALengthBefore = widgetA.length;
      await stopA();

      await writer.query(`INSERT INTO ${table} (id, title, status, score) VALUES (64002, 'second', 'open', 2.0)`);
      await waitFor(() => snapshotHasId(widgetB, 64002));

      assert.equal(widgetA.length, widgetALengthBefore);
      assert.deepEqual(latestIds(widgetB), [64001, 64002]);

      await stopB();
    } finally {
      await dropTable(client, table);
      await shutdownClient(writer);
    }
  });

  test('final widget unsubscribe stops all later deliveries', async () => {
    const table = `${ns}.${uniqueName('widgets_final')}`;
    const writer = await connectJwtClient();

    await client.query(
      `CREATE TABLE IF NOT EXISTS ${table} (
        id INT PRIMARY KEY,
        title TEXT NOT NULL,
        status TEXT NOT NULL,
        score DOUBLE
      )`,
    );

    const widgetA = [];
    const widgetB = [];
    const sql = `SELECT id, title, status, score FROM ${table}`;

    try {
      const stopA = await client.live(sql, (rows) => widgetA.push(sortRowsById(rows.map(mapIssueRow))), { lastRows: 0 });
      const stopB = await client.live(sql, (rows) => widgetB.push(sortRowsById(rows.map(mapIssueRow))), { lastRows: 0 });

      await writer.query(`INSERT INTO ${table} (id, title, status, score) VALUES (65001, 'first', 'open', 1.0)`);
      await waitFor(() => snapshotHasId(widgetA, 65001) && snapshotHasId(widgetB, 65001));

      const countBeforeStop = widgetA.length + widgetB.length;
      await stopA();
      await stopB();
      assert.equal(client.getSubscriptionCount(), 0);

      await writer.query(`INSERT INTO ${table} (id, title, status, score) VALUES (65002, 'after-stop', 'closed', 4.0)`);
      await sleep(300);

      assert.equal(widgetA.length + widgetB.length, countBeforeStop);
    } finally {
      await dropTable(client, table);
      await shutdownClient(writer);
    }
  });

  test('lazy browser-style client stays on HTTP until a live widget mounts', async () => {
    const table = `${ns}.${uniqueName('lazy_boot')}`;
    const lazyClient = createClient({
      url: SERVER_URL,
      authProvider: jwtAuthProvider(),
      wsLazyConnect: true,
    });

    try {
      await client.query(`CREATE TABLE IF NOT EXISTS ${table} (id INT PRIMARY KEY, title TEXT NOT NULL)`);
      await lazyClient.initialize();
      assert.equal(lazyClient.isConnected(), false, 'lazy client should not open websocket on initialize');

      const initialCount = await lazyClient.queryOne(`SELECT COUNT(*) AS total FROM ${table}`);
      assert.equal(initialCount.total.asInt(), 0);
      assert.equal(lazyClient.isConnected(), false, 'query path should stay HTTP-only before live mount');

      const events = [];
      const stop = await lazyClient.liveEvents(`SELECT * FROM ${table}`, (event) => events.push(event));
      await waitFor(() => events.some((event) => event.type === 'subscription_ack'));
      assert.equal(lazyClient.isConnected(), true, 'live mount should open websocket');

      await stop();
    } finally {
      await dropTable(client, table);
      await shutdownClient(lazyClient);
    }
  });

  test('detail queries keep working while the live transport disconnects and reconnects', async () => {
    const table = `${ns}.${uniqueName('detail_reconnect')}`;
    const writer = await connectJwtClient();

    await client.query(`CREATE TABLE IF NOT EXISTS ${table} (id INT PRIMARY KEY, title TEXT NOT NULL)`);

    const events = [];
    try {
      const stop = await client.liveEvents(`SELECT * FROM ${table}`, (event) => events.push(event), { lastRows: 0 });
      await waitFor(() => events.some((event) => event.type === 'subscription_ack'));

      await writer.query(`INSERT INTO ${table} (id, title) VALUES (67001, 'before-disconnect')`);
      await waitFor(() => events.some((event) => event.rows?.some?.((row) => row.id?.asInt?.() === 67001)));

      await client.disconnect();
      assert.equal(client.isConnected(), false);

      const detailDuringGap = await client.queryOne(`SELECT id, title FROM ${table} WHERE id = 67001`);
      assert.equal(detailDuringGap.id.asInt(), 67001);
      assert.equal(detailDuringGap.title.asString(), 'before-disconnect');

      const resumedEvents = [];
      const resumedStop = await client.liveEvents(`SELECT * FROM ${table}`, (event) => resumedEvents.push(event), { lastRows: 0 });
      await waitFor(() => resumedEvents.some((event) => event.type === 'subscription_ack'));

      await writer.query(`INSERT INTO ${table} (id, title) VALUES (67002, 'after-reconnect')`);
      await waitFor(() => resumedEvents.some((event) => event.rows?.some?.((row) => row.id?.asInt?.() === 67002)));

      await resumedStop();
      await stop();
    } finally {
      await dropTable(client, table);
      await shutdownClient(writer);
    }
  });

  test('basic auth session can refresh tokens while query and live views remain usable', async () => {
    const table = `${ns}.${uniqueName('auth_refresh')}`;
    const writer = await connectJwtClient();
    const basicClient = createClient({
      url: SERVER_URL,
      authProvider: async () => Auth.basic(ADMIN_USER, ADMIN_PASS),
      wsLazyConnect: true,
    });

    await client.query(`CREATE TABLE IF NOT EXISTS ${table} (id INT PRIMARY KEY, title TEXT NOT NULL)`);

    try {
      const login = await basicClient.login();
      assert.ok(login.access_token);

      const snapshots = [];
      const stop = await basicClient.live(
        `SELECT id, title FROM ${table}`,
        (rows) => snapshots.push(sortRowsById(rows.map((row) => ({ id: row.id.asInt(), title: row.title.asString() })))),
        { lastRows: 0 },
      );

      await writer.query(`INSERT INTO ${table} (id, title) VALUES (68001, 'first-live-row')`);
      await waitFor(() => snapshotHasId(snapshots, 68001));

      const refreshed = await basicClient.refreshToken(login.refresh_token);
      assert.ok(refreshed.access_token);

      const detail = await basicClient.queryOne(`SELECT id, title FROM ${table} WHERE id = 68001`);
      assert.equal(detail.id.asInt(), 68001);

      await writer.query(`INSERT INTO ${table} (id, title) VALUES (68002, 'after-refresh')`);
      await waitFor(() => snapshotHasId(snapshots, 68002));
      assert.deepEqual(latestIds(snapshots), [68001, 68002]);

      await stop();
    } finally {
      await dropTable(client, table);
      await shutdownClient(writer);
      await shutdownClient(basicClient);
    }
  });

  test('list and detail views stay consistent across insert, update, and delete', async () => {
    const table = `${ns}.${uniqueName('list_detail')}`;
    const writer = await connectJwtClient();

    await client.query(
      `CREATE TABLE IF NOT EXISTS ${table} (
        id INT PRIMARY KEY,
        title TEXT NOT NULL,
        status TEXT NOT NULL,
        score DOUBLE
      )`,
    );

    const snapshots = [];
    try {
      const stop = await client.live(
        `SELECT id, title, status, score FROM ${table}`,
        (rows) => snapshots.push(sortRowsById(rows.map(mapIssueRow))),
        { lastRows: 0 },
      );

      await writer.query(`INSERT INTO ${table} (id, title, status, score) VALUES (69001, 'created', 'open', 1.25)`);
      await waitFor(() => snapshotHasId(snapshots, 69001));

      let detail = await client.queryOne(`SELECT id, title, status, score FROM ${table} WHERE id = 69001`);
      assert.equal(detail.title.asString(), 'created');
      assert.equal(detail.status.asString(), 'open');

      await writer.query(`UPDATE ${table} SET title = 'updated', status = 'closed', score = 8.5 WHERE id = 69001`);
      await waitFor(() => latestRows(snapshots).some((row) => row.id === 69001 && row.status === 'closed'));

      detail = await client.queryOne(`SELECT id, title, status, score FROM ${table} WHERE id = 69001`);
      assert.equal(detail.title.asString(), 'updated');
      assert.equal(detail.status.asString(), 'closed');
      assert.equal(detail.score.asFloat(), 8.5);

      await writer.query(`DELETE FROM ${table} WHERE id = 69001`);
      await waitFor(() => latestRows(snapshots).every((row) => row.id !== 69001));

      detail = await client.queryOne(`SELECT id FROM ${table} WHERE id = 69001`);
      assert.equal(detail, null);

      await stop();
    } finally {
      await dropTable(client, table);
      await shutdownClient(writer);
    }
  });

  test('two page clients stay isolated when one page disconnects and later remounts', async () => {
    const table = `${ns}.${uniqueName('two_pages')}`;
    const pageA = await connectJwtClient();
    const pageB = await connectJwtClient();
    const writer = await connectJwtClient();

    await client.query(`CREATE TABLE IF NOT EXISTS ${table} (id INT PRIMARY KEY, title TEXT NOT NULL)`);

    const pageASnapshots = [];
    const pageBSnapshots = [];

    try {
      const stopA = await pageA.live(
        `SELECT id, title FROM ${table}`,
        (rows) => pageASnapshots.push(sortRowsById(rows.map((row) => ({ id: row.id.asInt(), title: row.title.asString() })))),
        { lastRows: 0 },
      );
      const stopB = await pageB.live(
        `SELECT id, title FROM ${table}`,
        (rows) => pageBSnapshots.push(sortRowsById(rows.map((row) => ({ id: row.id.asInt(), title: row.title.asString() })))),
        { lastRows: 0 },
      );

      await writer.query(`INSERT INTO ${table} (id, title) VALUES (70001, 'shared-first')`);
      await waitFor(() => snapshotHasId(pageASnapshots, 70001) && snapshotHasId(pageBSnapshots, 70001));

      await stopA();
      await pageA.disconnect();

      const pageALengthBefore = pageASnapshots.length;
      await writer.query(`INSERT INTO ${table} (id, title) VALUES (70002, 'while-a-away')`);
      await waitFor(() => snapshotHasId(pageBSnapshots, 70002));
      assert.equal(pageASnapshots.length, pageALengthBefore);

      const stopA2 = await pageA.live(
        `SELECT id, title FROM ${table}`,
        (rows) => pageASnapshots.push(sortRowsById(rows.map((row) => ({ id: row.id.asInt(), title: row.title.asString() })))),
        { lastRows: 0 },
      );

      await writer.query(`INSERT INTO ${table} (id, title) VALUES (70003, 'after-remount')`);
      await waitFor(() => snapshotHasId(pageASnapshots, 70003) && snapshotHasId(pageBSnapshots, 70003));

      assert.ok(!latestRows(pageASnapshots).some((row) => row.id === 70002), 'page A should not receive the disconnected gap row on remount');
      assert.ok(latestRows(pageBSnapshots).some((row) => row.id === 70002), 'page B should keep the gap row it stayed connected for');

      await stopA2();
      await stopB();
    } finally {
      await dropTable(client, table);
      await shutdownClient(writer);
      await shutdownClient(pageA);
      await shutdownClient(pageB);
    }
  });
});