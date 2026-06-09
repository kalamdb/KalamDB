import assert from 'node:assert/strict';
import test from 'node:test';

import { drizzle } from 'drizzle-orm/pg-proxy';
import { text } from 'drizzle-orm/pg-core';
import { Auth, createClient } from '@kalamdb/client';
import { kalamDriver, kTable, liveTable } from '../dist/index.js';

function createFakeWasmClient() {
  let connected = false;
  let nextSubscriptionId = 0;

  return {
    defaultNamespace: undefined,
    queryCalls: [],
    liveCalls: [],
    setAuthProvider() {},
    setDefaultNamespace(namespace) {
      this.defaultNamespace = namespace ?? undefined;
    },
    setWsLazyConnect() {},
    setAutoReconnect() {},
    onConnect() {},
    onDisconnect() {},
    onError() {},
    onReceive() {},
    onSend() {},
    isConnected() {
      return connected;
    },
    async connect() {
      connected = true;
    },
    async disconnect() {
      connected = false;
    },
    async query(sql) {
      this.queryCalls.push({ sql });
      return JSON.stringify({
        status: 'success',
        results: [{ row_count: 0, named_rows: [] }],
      });
    },
    async queryWithParams(sql, paramsJson) {
      this.queryCalls.push({ sql, paramsJson });
      return JSON.stringify({
        status: 'success',
        results: [{ row_count: 0, named_rows: [] }],
      });
    },
    async live(sql, optionsJson, _callback) {
      nextSubscriptionId += 1;
      this.liveCalls.push({ sql, optionsJson });
      return `live-${nextSubscriptionId}`;
    },
    async unsubscribe() {},
    getSubscriptions() {
      return '[]';
    },
  };
}

function createNamespacedOrmClient(namespace = 'workspace') {
  const client = createClient({
    url: 'http://127.0.0.1:2900',
    namespace,
    authProvider: async () => Auth.jwt('orm-test-token'),
  });
  const fakeWasmClient = createFakeWasmClient();
  client.initialized = true;
  client.wasmClient = fakeWasmClient;
  client.attachWasmClientState();
  return { client, fakeWasmClient };
}

test('kalamDriver forwards the client namespace for unqualified tables', async () => {
  const { client, fakeWasmClient } = createNamespacedOrmClient();
  const db = drizzle(kalamDriver(client));
  const items = kTable('items', {
    id: text('id'),
    name: text('name'),
  });

  await db.select().from(items);

  assert.equal(fakeWasmClient.defaultNamespace, 'workspace');
  assert.match(fakeWasmClient.queryCalls[0].sql, /from\s+"?items"?/i);
});

test('liveTable reuses the client namespace for unqualified tables', async () => {
  const { client, fakeWasmClient } = createNamespacedOrmClient();
  const items = kTable('items', {
    id: text('id'),
    name: text('name'),
  });

  const stop = await liveTable(client, items, () => {});

  assert.equal(fakeWasmClient.defaultNamespace, 'workspace');
  assert.match(fakeWasmClient.liveCalls[0].sql, /from\s+"?items"?/i);

  await stop();
});
