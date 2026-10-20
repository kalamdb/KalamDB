import assert from 'node:assert/strict';
import test from 'node:test';
import { LiveQueryController, createLiveQueryDescriptor } from '../dist/src/index.js';

function createFakeClient() {
  const calls = [];
  let unsubscribeCount = 0;

  return {
    calls,
    get unsubscribeCount() {
      return unsubscribeCount;
    },
    async live(sql, callback, options = {}) {
      calls.push({ sql, callback, options });
      return async () => {
        unsubscribeCount += 1;
      };
    },
  };
}

test('LiveQueryController projects snapshots and supports refetch', async () => {
  const client = createFakeClient();
  const descriptor = createLiveQueryDescriptor({
    mode: 'sql',
    sourceSql: 'SELECT * FROM chat.messages ORDER BY created_at DESC LIMIT 1',
    subscriptionSql: 'SELECT * FROM chat.messages',
    projection: {
      orderBy: [{ column: 'created_at', direction: 'desc' }],
      limit: 1,
    },
    getKey: 'id',
  });
  const controller = new LiveQueryController(client, descriptor);
  const snapshots = [];

  controller.subscribe((snapshot) => snapshots.push(snapshot));
  await controller.start();
  client.calls[0].callback([
    { id: 'a', created_at: 1 },
    { id: 'b', created_at: 2 },
  ]);

  assert.equal(client.calls[0].sql, 'SELECT * FROM chat.messages');
  assert.deepEqual(snapshots.at(-1).rows, [{ id: 'b', created_at: 2 }]);
  assert.equal(snapshots.at(-1).status, 'live');

  await controller.refetch();
  assert.equal(client.unsubscribeCount, 1);
  assert.equal(client.calls.length, 2);
});

test('obsolete errors and checkpoints do not mutate a new or disposed session', async () => {
  const client = createFakeClient();
  let errors = 0;
  let checkpoints = 0;
  const controller = new LiveQueryController(client,
    createLiveQueryDescriptor({mode: 'sql', sourceSql: 'SELECT * FROM messages'}),
    {onError: () => errors++, onCheckpoint: () => checkpoints++});
  await controller.start();
  const old = client.calls[0];
  await controller.refetch();
  old.options.onError({message: 'old'});
  old.options.onCheckpoint({lastVersionId: '1'});
  assert.equal(controller.getSnapshot().status, 'loading');
  await controller.dispose();
  client.calls[1].options.onError({message: 'late'});
  client.calls[1].options.onCheckpoint({lastVersionId: '2'});
  assert.equal(controller.getSnapshot().status, 'disposed');
  assert.equal(errors, 0);
  assert.equal(checkpoints, 0);
  await assert.rejects(controller.start(), /disposed/);
  assert.equal(client.calls.length, 2);
});

test('disposing during teardown cannot launch a replacement subscription', async () => {
  let release;
  const client = {calls: 0, async live() {
    this.calls++;
    return () => new Promise(resolve => {release = resolve;});
  }};
  const controller = new LiveQueryController(client,
    createLiveQueryDescriptor({mode: 'sql', sourceSql: 'SELECT * FROM messages'}));
  await controller.start();
  const restarting = controller.refetch();
  await controller.dispose();
  release();
  await restarting;
  assert.equal(client.calls, 1);
  assert.equal(controller.getSnapshot().status, 'disposed');
});
