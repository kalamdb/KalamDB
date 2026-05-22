import assert from 'node:assert/strict';
import test from 'node:test';

import {
  Auth,
  createConsumerClient,
} from '../dist/src/index.js';

function jsonResponse(body, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: {
      'Content-Type': 'application/json',
    },
  });
}

function readHeader(headers, name) {
  if (!headers) {
    return undefined;
  }
  if (typeof headers.get === 'function') {
    return headers.get(name) ?? undefined;
  }
  return headers[name] ?? headers[name.toLowerCase()];
}

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function makeLoginResponse(accessToken) {
  return {
    access_token: accessToken,
    refresh_token: `refresh-${accessToken}`,
    expires_at: '2026-03-08T10:00:00Z',
    user: {
      id: '1',
      username: 'worker',
      role: 'service',
      created_at: '',
      updated_at: '',
    },
  };
}

test('consumeBatch maps options to the topic HTTP API and decodes payloads', async () => {
  const originalFetch = globalThis.fetch;
  const calls = [];
  const payload = {
    id: 9,
    status: 'new',
    _table: 'orders',
  };

  globalThis.fetch = async (url, options) => {
    calls.push({ url, options });
    return jsonResponse({
      messages: [{
        topic_id: 'orders',
        partition_id: 2,
        offset: 9,
        key: '{"id":9}',
        user: 'alice',
        op: 'Insert',
        timestamp_ms: 1730000000000,
        payload: Buffer.from(JSON.stringify(payload), 'utf8').toString('base64'),
      }],
      next_offset: 10,
      has_more: false,
    });
  };

  try {
    const client = createConsumerClient({
      url: 'http://127.0.0.1:2900',
      authProvider: async () => Auth.jwt('jwt-123'),
    });

    const batch = await client.consumeBatch({
      topic: 'orders',
      group_id: 'billing',
      start: 'earliest',
      batch_size: 25,
      partition_id: 2,
      timeout_seconds: 15,
      auto_ack: false,
      concurrency_per_partition: 3,
    });

    assert.equal(batch.messages.length, 1);
    assert.equal(batch.messages[0].topic, 'orders');
    assert.equal(batch.messages[0].group_id, 'billing');
  assert.equal(batch.messages[0].key, '{"id":9}');
  assert.equal(batch.messages[0].op, 'Insert');
  assert.equal(batch.messages[0].timestamp_ms, 1730000000000);
  assert.equal(batch.messages[0].user, 'alice');
  assert.deepEqual(batch.messages[0].payload, payload);
  assert.deepEqual(batch.messages[0].value, payload);

    const request = calls[0];
    assert.equal(request.url, 'http://127.0.0.1:2900/v1/api/topics/consume');
    assert.equal(readHeader(request.options.headers, 'Authorization'), 'Bearer jwt-123');
    assert.deepEqual(JSON.parse(request.options.body), {
      topic_id: 'orders',
      group_id: 'billing',
      start: 'Earliest',
      limit: 25,
      partition_id: 2,
      timeout_seconds: 15,
    });
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test('consumeBatch accepts user_id in consume responses', async () => {
  const originalFetch = globalThis.fetch;

  globalThis.fetch = async () => jsonResponse({
    messages: [{
      topic_id: 'orders',
      partition_id: 0,
      offset: 4,
      user_id: 'producer-42',
      payload: Buffer.from(JSON.stringify({ id: 4, status: 'updated' }), 'utf8').toString('base64'),
    }],
    next_offset: 5,
    has_more: false,
  });

  try {
    const client = createConsumerClient({
      url: 'http://127.0.0.1:2900',
      authProvider: async () => Auth.jwt('jwt-123'),
    });

    const batch = await client.consumeBatch({
      topic: 'orders',
      group_id: 'billing',
    });

    assert.equal(batch.messages.length, 1);
    assert.equal(batch.messages[0].user, 'producer-42');
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test('ack exchanges basic auth once and reuses the cached JWT for later requests', async () => {
  const originalFetch = globalThis.fetch;
  const calls = [];

  globalThis.fetch = async (url, options) => {
    calls.push({ url, options });
    if (url.endsWith('/v1/api/auth/login')) {
      return jsonResponse({
        access_token: 'worker-jwt',
        refresh_token: 'refresh-123',
        expires_at: '2026-03-08T10:00:00Z',
        user: { id: '1', username: 'worker', role: 'service', created_at: '', updated_at: '' },
      });
    }
    return jsonResponse({
      success: true,
      acknowledged_offset: JSON.parse(options.body).upto_offset,
    });
  };

  try {
    const client = createConsumerClient({
      url: 'http://127.0.0.1:2900',
      authProvider: async () => Auth.basic('worker', 'secret'),
    });

    await client.ack('orders', 'billing', 0, 42);
    await client.ack('orders', 'billing', 0, 43);

    const loginCalls = calls.filter((call) => call.url.endsWith('/v1/api/auth/login'));
    const ackCalls = calls.filter((call) => call.url.endsWith('/v1/api/topics/ack'));

    assert.equal(loginCalls.length, 1);
    assert.equal(ackCalls.length, 2);
    assert.equal(readHeader(ackCalls[0].options.headers, 'Authorization'), 'Bearer worker-jwt');
    assert.equal(readHeader(ackCalls[1].options.headers, 'Authorization'), 'Bearer worker-jwt');
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test('consumer run preserves the latest cursor after an empty poll', async () => {
  const originalFetch = globalThis.fetch;
  const consumeBodies = [];
  const ackBodies = [];
  let consumeCallCount = 0;

  globalThis.fetch = async (url, options) => {
    if (url.endsWith('/v1/api/topics/consume')) {
      const body = JSON.parse(options.body);
      consumeBodies.push(body);
      consumeCallCount += 1;

      if (consumeCallCount === 1) {
        return jsonResponse({
          messages: [],
          next_offset: 41,
          has_more: false,
        });
      }

      if (consumeCallCount === 2) {
        return jsonResponse({
          messages: [{
            topic_id: 'events',
            partition_id: 0,
            offset: 41,
            user: 'reader-1',
            payload: Buffer.from(JSON.stringify({ status: 'live', _table: 'events' }), 'utf8').toString('base64'),
          }],
          next_offset: 42,
          has_more: false,
        });
      }

      throw new Error(`Unexpected consume call ${consumeCallCount}`);
    }

    ackBodies.push(JSON.parse(options.body));
    return jsonResponse({
      success: true,
      acknowledged_offset: ackBodies[ackBodies.length - 1].upto_offset,
    });
  };

  try {
    const client = createConsumerClient({
      url: 'http://127.0.0.1:2900',
      authProvider: async () => Auth.jwt('jwt-123'),
    });

    const handle = client.consumer({
      topic: 'events',
      group_id: 'latest-reader',
      start: 'latest',
    });

    let calls = 0;
    await handle.run(async (ctx) => {
      calls += 1;
      assert.equal(ctx.message.offset, 41);
      assert.deepEqual(ctx.message.payload, { status: 'live', _table: 'events' });
      assert.deepEqual(ctx.message.value, { status: 'live', _table: 'events' });
      await ctx.ack();
      handle.stop();
    });

    assert.equal(calls, 1);
    assert.deepEqual(consumeBodies, [
      {
        topic_id: 'events',
        group_id: 'latest-reader',
        start: 'Latest',
        limit: 10,
        partition_id: 0,
      },
      {
        topic_id: 'events',
        group_id: 'latest-reader',
        start: { Offset: 41 },
        limit: 10,
        partition_id: 0,
      },
    ]);
    assert.deepEqual(ackBodies, [{
      topic_id: 'events',
      group_id: 'latest-reader',
      partition_id: 0,
      upto_offset: 41,
    }]);
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test('consumer run supports manual and auto acknowledgments', async () => {
  const originalFetch = globalThis.fetch;
  const consumeResponses = [
    {
      messages: [{
        topic_id: 'events',
        partition_id: 0,
        offset: 20,
        user: 'manual-user',
        payload: Buffer.from(JSON.stringify({ status: 'manual' }), 'utf8').toString('base64'),
      }],
      next_offset: 21,
      has_more: true,
    },
    {
      messages: [{
        topic_id: 'events',
        partition_id: 1,
        offset: 30,
        user: 'auto-user',
        payload: Buffer.from(JSON.stringify({ status: 'auto' }), 'utf8').toString('base64'),
      }],
      next_offset: 31,
      has_more: true,
    },
  ];
  const ackBodies = [];

  globalThis.fetch = async (url, options) => {
    if (url.endsWith('/v1/api/topics/consume')) {
      return jsonResponse(consumeResponses.shift() ?? {
        messages: [],
        next_offset: 0,
        has_more: false,
      });
    }

    ackBodies.push(JSON.parse(options.body));
    return jsonResponse({
      success: true,
      acknowledged_offset: ackBodies[ackBodies.length - 1].upto_offset,
    });
  };

  try {
    const client = createConsumerClient({
      url: 'http://127.0.0.1:2900',
      authProvider: async () => Auth.jwt('jwt-123'),
    });

    const manualHandle = client.consumer({
      topic: 'events',
      group_id: 'manual-reader',
      start: 'latest',
      batch_size: 1,
    });

    let manualCalls = 0;
    await manualHandle.run(async (ctx) => {
      manualCalls += 1;
      assert.equal(ctx.message.offset, 20);
      assert.deepEqual(ctx.message.payload, { status: 'manual' });
      await ctx.ack();
      manualHandle.stop();
    });

    const autoHandle = client.consumer({
      topic: 'events',
      group_id: 'auto-reader',
      auto_ack: true,
      partition_id: 1,
    });

    let autoCalls = 0;
    await autoHandle.run(async (ctx) => {
      autoCalls += 1;
      assert.equal(ctx.message.partition_id, 1);
      assert.equal(ctx.user, 'auto-user');
      assert.deepEqual(ctx.message.payload, { status: 'auto' });
      autoHandle.stop();
    });

    assert.equal(manualCalls, 1);
    assert.equal(autoCalls, 1);
    assert.deepEqual(ackBodies, [
      {
        topic_id: 'events',
        group_id: 'manual-reader',
        partition_id: 0,
        upto_offset: 20,
      },
      {
        topic_id: 'events',
        group_id: 'auto-reader',
        partition_id: 1,
        upto_offset: 30,
      },
    ]);
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test('consumeBatch rejects consume responses missing user metadata', async () => {
  const originalFetch = globalThis.fetch;

  globalThis.fetch = async () => jsonResponse({
    messages: [{
      topic_id: 'orders',
      partition_id: 0,
      offset: 4,
      payload: Buffer.from(JSON.stringify({ id: 4, status: 'updated' }), 'utf8').toString('base64'),
    }],
    next_offset: 5,
    has_more: false,
  });

  try {
    const client = createConsumerClient({
      url: 'http://127.0.0.1:2900',
      authProvider: async () => Auth.jwt('jwt-123'),
    });

    await assert.rejects(
      () => client.consumeBatch({
        topic: 'orders',
        group_id: 'billing',
      }),
      /missing required user metadata/i,
    );
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test('consumeBatch retries once with refreshed JWT auth after TOKEN_EXPIRED', async () => {
  let authCalls = 0;
  const authHeaders = [];

  const client = createConsumerClient({
    url: 'http://127.0.0.1:2900',
    authProvider: async () => Auth.jwt(authCalls++ === 0 ? 'jwt-stale' : 'jwt-fresh'),
  });

  client.topicTransport = {
    consume: async (authHeader) => {
      authHeaders.push(authHeader);
      if (authHeaders.length === 1) {
        throw { status: 401, code: 'TOKEN_EXPIRED', message: 'jwt expired' };
      }

      return {
        messages: [{
          topic: 'orders',
          group_id: 'billing',
          partition_id: 0,
          offset: 12,
          user: 'worker-1',
          payload: { id: 12, status: 'retried' },
        }],
        next_offset: 13,
        has_more: false,
      };
    },
    ack: async () => {
      throw new Error('unexpected ack');
    },
  };

  const batch = await client.consumeBatch({
    topic: 'orders',
    group_id: 'billing',
  });

  assert.equal(authCalls, 2);
  assert.deepEqual(authHeaders, ['Bearer jwt-stale', 'Bearer jwt-fresh']);
  assert.equal(batch.messages.length, 1);
  assert.equal(batch.messages[0].offset, 12);
  assert.deepEqual(batch.messages[0].payload, { id: 12, status: 'retried' });
});

test('ack retries once with a fresh login when topic auth becomes UNAUTHENTICATED', async () => {
  let loginCalls = 0;
  const authHeaders = [];

  const client = createConsumerClient({
    url: 'http://127.0.0.1:2900',
    authProvider: async () => Auth.basic('worker', 'secret'),
  });

  client.sqlClient.login = async () => makeLoginResponse(loginCalls++ === 0 ? 'jwt-one' : 'jwt-two');
  client.topicTransport = {
    consume: async () => {
      throw new Error('unexpected consume');
    },
    ack: async (authHeader) => {
      authHeaders.push(authHeader);
      if (authHeaders.length === 1) {
        throw { status: 401, code: 'UNAUTHENTICATED', message: 'token expired' };
      }

      return {
        success: true,
        acknowledged_offset: 44,
      };
    },
  };

  const response = await client.ack('orders', 'billing', 0, 44);

  assert.equal(loginCalls, 2);
  assert.deepEqual(authHeaders, ['Bearer jwt-one', 'Bearer jwt-two']);
  assert.deepEqual(response, {
    success: true,
    acknowledged_offset: 44,
  });
});

test('consumeBatch does not retry non-retriable topic authorization failures', async () => {
  let authCalls = 0;
  let requestCalls = 0;

  const client = createConsumerClient({
    url: 'http://127.0.0.1:2900',
    authProvider: async () => Auth.jwt(`jwt-${++authCalls}`),
  });

  client.topicTransport = {
    consume: async () => {
      requestCalls += 1;
      throw { status: 403, code: 'FORBIDDEN', message: 'group denied' };
    },
    ack: async () => {
      throw new Error('unexpected ack');
    },
  };

  await assert.rejects(
    () => client.consumeBatch({ topic: 'orders', group_id: 'billing' }),
    /group denied/i,
  );

  assert.equal(authCalls, 1);
  assert.equal(requestCalls, 1);
});

test('consumer run keeps at most one consume request in flight and exits cleanly after stop', async () => {
  const client = createConsumerClient({
    url: 'http://127.0.0.1:2900',
    authProvider: async () => Auth.jwt('jwt-123'),
  });

  let consumeCalls = 0;
  const pendingPoll = deferred();

  client.consumeBatch = async () => {
    consumeCalls += 1;
    return pendingPoll.promise;
  };

  const handle = client.consumer({
    topic: 'events',
    group_id: 'worker',
  });

  const runPromise = handle.run(async () => {
    throw new Error('handler should not run for an empty batch');
  });

  await Promise.resolve();
  assert.equal(consumeCalls, 1);

  handle.stop();
  pendingPoll.resolve({
    messages: [],
    next_offset: 5,
    has_more: false,
  });

  await runPromise;
  assert.equal(consumeCalls, 1);
});

test('consumer run stops after the current message even when the server reports has_more', async () => {
  const client = createConsumerClient({
    url: 'http://127.0.0.1:2900',
    authProvider: async () => Auth.jwt('jwt-123'),
  });

  let consumeCalls = 0;
  const ackedOffsets = [];

  client.consumeBatch = async () => {
    consumeCalls += 1;
    if (consumeCalls > 1) {
      throw new Error('consumer fetched again after stop');
    }

    return {
      messages: [{
        topic: 'events',
        group_id: 'worker',
        partition_id: 0,
        offset: 10,
        user: 'processor',
        payload: { id: 10, status: 'ready' },
        value: { id: 10, status: 'ready' },
      }],
      next_offset: 11,
      has_more: true,
    };
  };
  client.ack = async (_topic, _groupId, _partitionId, uptoOffset) => {
    ackedOffsets.push(uptoOffset);
    return { success: true, acknowledged_offset: uptoOffset };
  };

  const handle = client.consumer({
    topic: 'events',
    group_id: 'worker',
    auto_ack: true,
  });

  let seenOffset = 0;
  await handle.run(async (ctx) => {
    seenOffset = ctx.message.offset;
    handle.stop();
  });

  assert.equal(seenOffset, 10);
  assert.equal(consumeCalls, 1);
  assert.deepEqual(ackedOffsets, [10]);
});