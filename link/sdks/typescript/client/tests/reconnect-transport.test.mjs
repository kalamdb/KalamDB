import assert from 'node:assert/strict';
import { once } from 'node:events';
import { setTimeout as delay } from 'node:timers/promises';
import test from 'node:test';
import { WebSocketServer } from 'ws';

import { Auth, KalamDBClient } from '../dist/src/index.js';

const authSuccess = {
  type: 'auth_success',
  user: 'fixture',
  role: 'user',
  protocol: { serialization: 'json', compression: 'none' },
};

function ack(socket, id) {
  socket.send(JSON.stringify({
    type: 'subscription_ack',
    subscription_id: id,
    total_rows: 0,
    schema: [],
    batch_control: { status: 'ready', batch_num: 0, has_more: false },
  }));
}

function insert(socket, id, versions) {
  socket.send(JSON.stringify({
    type: 'change',
    change_type: 'insert',
    subscription_id: id,
    rows: versions.map(version => ({ id: String(version), _version: version })),
  }));
}

async function until(read) {
  for (let i = 0; i < 250; i++) {
    const value = read();
    if (value) return value;
    await delay(5);
  }
  assert.fail('Timed out waiting for WebSocket fixture');
}

async function fixture(t, onSocket) {
  const server = new WebSocketServer({ host: '127.0.0.1', port: 0 });
  await once(server, 'listening');
  server.on('connection', onSocket);
  t.after(async () => {
    for (const socket of server.clients) socket.terminate();
    await new Promise(resolve => server.close(resolve));
  });
  return `http://127.0.0.1:${server.address().port}`;
}

test('recovers one subscribed socket with fresh credentials and a consumed cursor', async t => {
  const sockets = [];
  const offeredTokens = [];
  const subscriptions = [];
  const url = await fixture(t, (socket, request) => {
    sockets.push(socket);
    offeredTokens.push(request.headers['sec-websocket-protocol']);
    socket.send(JSON.stringify(authSuccess));
    socket.on('message', data => {
      const frame = JSON.parse(data);
      if (frame.type === 'subscribe') subscriptions.push(frame.subscription);
    });
  });
  let token = 'fixture-token-1';
  const client = new KalamDBClient({ url, authProvider: async () => Auth.jwt(token) });
  t.after(() => client.shutdown());

  await client.connect();
  client.setReconnectDelay(10, 10);
  const inserts = [];
  const subscribe = client.liveEvents('SELECT id FROM fixture', event => {
    if (event.type === 'change' && event.change_type === 'insert') {
      inserts.push(...event.rows.map(row => row.id.asString()));
    }
  });
  const initial = await until(() => subscriptions[0]);
  ack(sockets[0], initial.id);
  await subscribe;
  insert(sockets[0], initial.id, [42]);
  await until(() => inserts.length === 1);

  token = 'fixture-token-2';
  sockets[0].close(1001, 'server restart');
  const resumed = await until(() => subscriptions[1]);
  assert.equal(resumed.id, initial.id);
  assert.equal(resumed.options.from, '42');
  assert.match(offeredTokens[1], /fixture-token-2$/);
  assert.equal(client.isConnected(), false);

  const joined = client.connect();
  ack(sockets[1], resumed.id);
  await joined;
  assert.equal(sockets.length, 2);
  assert.equal(client.isConnected(), true);
  insert(sockets[1], resumed.id, [42, 43]);
  await until(() => inserts.length === 2);
  assert.deepEqual(inserts, ['42', '43']);

  sockets[1].terminate();
  const afterDrop = await until(() => subscriptions[2]);
  assert.equal(afterDrop.options.from, '43');
  ack(sockets[2], afterDrop.id);
  await until(() => client.isConnected());
  sockets[2].send(JSON.stringify({
    type: 'error', subscription_id: afterDrop.id,
    code: 'UNSUPPORTED', message: 'complete-event history expired; resynchronize',
  }));
  const snapshot = await until(() => subscriptions[3]);
  assert.equal(snapshot.id, initial.id);
  assert.equal(snapshot.options.from, undefined);
  ack(sockets[2], snapshot.id);
  insert(sockets[2], snapshot.id, [7]);
  await until(() => inserts.length === 3);
  assert.deepEqual(inserts, ['42', '43', '7']);

});

test('close during authentication rejects connect and honors disabled retries', async t => {
  const sockets = [];
  const url = await fixture(t, socket => {
    sockets.push(socket);
    if (sockets.length === 1) socket.close(1000, 'closed before authentication');
    else socket.send(JSON.stringify(authSuccess));
  });
  const client = new KalamDBClient({
    url,
    authProvider: async () => Auth.jwt('fixture-token'),
  });
  t.after(() => client.shutdown());

  await assert.rejects(Promise.race([
    client.connect(),
    delay(800).then(() => { throw new Error('fixture did not settle'); }),
  ]), error => String(error).includes('closed before authentication'));
  await client.connect();
  client.setAutoReconnect(false);
  // Calling connect on a ready client must keep the configured opt-out.
  await client.connect();
  client.setReconnectDelay(10, 10);
  sockets[1].close(1001, 'server restart');
  await until(() => !client.isConnected());
  await delay(50);
  assert.equal(sockets.length, 2);
  await client.connect();
  assert.equal(sockets.length, 3);
});


test('manual disconnect cancels scheduled recovery and permits a new connection', async t => {
  const sockets = [];
  const url = await fixture(t, socket => {
    sockets.push(socket);
    socket.send(JSON.stringify(authSuccess));
  });
  const client = new KalamDBClient({ url, authProvider: async () => Auth.jwt('fixture-token') });
  t.after(() => client.shutdown());

  await client.connect();
  client.setReconnectDelay(80, 80);
  sockets[0].close(1001, 'server restart');
  await until(() => client.isReconnecting());
  await client.disconnect();
  await delay(120);
  assert.equal(sockets.length, 1);
  assert.equal(client.isReconnecting(), false);
  await client.connect();
  assert.equal(sockets.length, 2);
});
