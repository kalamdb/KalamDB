/**
 * Production resilience test suite for runConsumer / runAgent.
 *
 * These tests cover scenarios encountered under real production load:
 * manual acks, short-circuit retry, connection budget exhaustion,
 * changeParser filtering, stopSignal clean shutdown, custom runKeyFactory,
 * and the empty-poll recovery path for onConnectionRestored.
 *
 * Each test is isolated and runs against the compiled dist/ output.
 */

import assert from 'node:assert/strict';
import test from 'node:test';

import { runConsumer } from '../dist/src/index.js';

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

function makeMessage(overrides = {}) {
  return {
    offset: 7,
    partition_id: 0,
    topic: 'blog.summarizer',
    group_id: 'blog-summarizer-agent',
    user: 'root',
    payload: { blog_id: '42', content: 'hello world', _table: 'blog.posts' },
    value: { blog_id: '42', content: 'hello world', _table: 'blog.posts' },
    ...overrides,
  };
}

/**
 * Mock client that supports the full ConsumerHandle interface including the
 * lifecycle hooks second argument to run(). Calling hooks?.onBatchSuccess?.()
 * before dispatching messages matches the real KalamConsumerClient behaviour.
 */
function createMockClient(messages, options = {}) {
  const state = {
    consumerOptions: [],
    ackedOffsets: [],
    queryCalls: [],
    runCount: 0,
  };

  const client = {
    query: async (sql, params) => {
      state.queryCalls.push({ sql, params });
      return { status: 'success', results: [] };
    },
    queryOne: async () => null,
    queryAll: async () => [],
    consumer: (consumeOptions) => {
      state.consumerOptions.push(consumeOptions);
      let stopped = false;

      return {
        run: async (handler, hooks) => {
          state.runCount += 1;
          // Fire onBatchSuccess before individual message dispatch —
          // this mirrors KalamConsumerClient which calls the hook once per
          // batch poll (including empty polls) before iterating messages.
          hooks?.onBatchSuccess?.({ nextOffset: 0, hasMore: false, messageCount: messages.length });

          const dispatchMessage = async (message) => {
            await handler({
              user: message.consumeCtxUser ?? message.user,
              message,
              ack: async () => {
                if (options.ackShouldThrow) {
                  throw new Error('ack failed');
                }
                state.ackedOffsets.push(message.offset);
              },
            });
          };

          for (const message of messages) {
            if (stopped) break;
            await dispatchMessage(message);
          }
        },
        stop: () => {
          stopped = true;
        },
      };
    },
  };

  return { client, state };
}

// ---------------------------------------------------------------------------
// Scenario 1 — Manual ctx.ack() inside onChange then successful return
// ---------------------------------------------------------------------------
test('manual ctx.ack inside onChange then successful return sends exactly one ack', async () => {
  const message = makeMessage({ offset: 201 });
  const { client, state } = createMockClient([message]);
  let calls = 0;

  await runConsumer({
    client,
    name: 'manual-ack-success',
    topic: message.topic,
    groupId: message.group_id,
    retry: { maxAttempts: 3, initialBackoffMs: 0, maxBackoffMs: 0 },
    onChange: async (ctx) => {
      calls += 1;
      await ctx.ack(); // explicit ack before returning normally
    },
  });

  assert.equal(calls, 1);
  // The runConsumer runtime also calls ack() on success, but the idempotent
  // guard must prevent a second network call — exactly one ack total.
  assert.deepEqual(state.ackedOffsets, [201]);
});

// ---------------------------------------------------------------------------
// Scenario 2 — Manual ctx.ack() inside onChange, then onChange throws
// ---------------------------------------------------------------------------
test('manual ctx.ack inside onChange then throw routes to onError not retry', async () => {
  const message = makeMessage({ offset: 202 });
  const { client, state } = createMockClient([message]);
  let calls = 0;
  let failedCalls = 0;
  const errors = [];

  await runConsumer({
    client,
    name: 'manual-ack-throw',
    topic: message.topic,
    groupId: message.group_id,
    // Plenty of retry budget — but acked=true must short-circuit retries.
    retry: { maxAttempts: 5, initialBackoffMs: 0, maxBackoffMs: 0 },
    onChange: async (ctx) => {
      calls += 1;
      await ctx.ack(); // optimistic ack before the downstream write
      throw new Error('downstream-write-failed');
    },
    onFailed: async () => {
      failedCalls += 1;
    },
    onError: (event) => {
      errors.push(String(event.error));
    },
  });

  // onChange ran exactly once — acked=true prevents retry
  assert.equal(calls, 1);
  // Ack was sent despite the thrown error
  assert.deepEqual(state.ackedOffsets, [202]);
  // Error surfaced to onError (not onFailed) because ack already completed
  assert.equal(errors.length, 1);
  assert.match(errors[0], /downstream-write-failed/);
  assert.equal(failedCalls, 0);
});

// ---------------------------------------------------------------------------
// Scenario 3 — Custom shouldRetry short-circuits retries
// Also validates the failedCtx.attempt bug-fix: attempt reflects actual
// tries made, not maxAttempts.
// ---------------------------------------------------------------------------
test('custom shouldRetry false skips all retries and reports actual attempt count', async () => {
  const message = makeMessage({ offset: 203 });
  const { client, state } = createMockClient([message]);
  let changes = 0;
  let failedAttempt = -1;
  const retries = [];

  await runConsumer({
    client,
    name: 'should-retry-false',
    topic: message.topic,
    groupId: message.group_id,
    retry: {
      maxAttempts: 5,
      initialBackoffMs: 0,
      maxBackoffMs: 0,
      // Non-retryable domain error — bail immediately
      shouldRetry: (error) => !String(error).includes('constraint-violation'),
    },
    onChange: async () => {
      changes += 1;
      throw new Error('constraint-violation: unique index on blog_id');
    },
    onRetry: (event) => retries.push(event.attempt),
    onFailed: async (ctx) => {
      failedAttempt = ctx.attempt;
    },
    ackOnFailed: true,
  });

  // shouldRetry returned false on attempt 1 → only 1 onChange call
  assert.equal(changes, 1);
  // onRetry must NOT fire because shouldRetry short-circuited
  assert.deepEqual(retries, []);
  // failedCtx.attempt must reflect the actual last attempt (1), not maxAttempts (5)
  assert.equal(failedAttempt, 1);
  // Message acked via ackOnFailed: true
  assert.deepEqual(state.ackedOffsets, [203]);
});

// ---------------------------------------------------------------------------
// Scenario 4 — ackOnFailed: false leaves the message unacked
// ---------------------------------------------------------------------------
test('ackOnFailed false leaves message unacked after onFailed succeeds', async () => {
  const message = makeMessage({ offset: 204 });
  const { client, state } = createMockClient([message]);
  let failedCalls = 0;

  await runConsumer({
    client,
    name: 'no-ack-on-failed',
    topic: message.topic,
    groupId: message.group_id,
    retry: { maxAttempts: 2, initialBackoffMs: 0, maxBackoffMs: 0 },
    onChange: async () => {
      throw new Error('permanent error');
    },
    onFailed: async () => {
      failedCalls += 1;
      // successfully writes to a dead-letter store but intentionally skips ack
      // so the message can be replayed from its current offset on next startup
    },
    ackOnFailed: false,
  });

  assert.equal(failedCalls, 1);
  // No ack must have been sent — the message cursor stays at offset 204
  assert.deepEqual(state.ackedOffsets, []);
});

// ---------------------------------------------------------------------------
// Scenario 5 — changeParser returning null: ack + skip, onChange not called
// ---------------------------------------------------------------------------
test('changeParser returning null acks message and skips onChange', async () => {
  const message = makeMessage({ offset: 205 });
  const { client, state } = createMockClient([message]);
  let calls = 0;
  const errors = [];

  await runConsumer({
    client,
    name: 'null-parser-skip',
    topic: message.topic,
    groupId: message.group_id,
    retry: { maxAttempts: 3, initialBackoffMs: 0, maxBackoffMs: 0 },
    // Production pattern: consumer watches a multi-table topic but only
    // processes specific _table values; null = ignore this message.
    changeParser: () => null,
    onChange: async () => {
      calls += 1;
    },
    onError: (event) => errors.push(String(event.error)),
  });

  // onChange must never be called for filtered messages
  assert.equal(calls, 0);
  assert.equal(errors.length, 0);
  // The message MUST be acked to advance the topic cursor, preventing
  // the consumer from re-reading filtered messages on restart.
  assert.deepEqual(state.ackedOffsets, [205]);
});

// ---------------------------------------------------------------------------
// Scenario 6 — connectionRetry.enabled: false — immediate throw, no retries
// ---------------------------------------------------------------------------
test('connectionRetry enabled false throws immediately on first connection error', async () => {
  let runs = 0;
  const client = {
    query: async () => ({ status: 'success', results: [] }),
    queryOne: async () => null,
    queryAll: async () => [],
    consumer: () => ({
      run: async () => {
        runs += 1;
        throw new Error('server-unavailable');
      },
      stop: () => {},
    }),
  };

  const connectionErrors = [];

  await assert.rejects(
    () =>
      runConsumer({
        client,
        name: 'no-retry-worker',
        topic: 'events',
        groupId: 'batch-job',
        // Batch jobs should fail-fast rather than loop forever
        connectionRetry: { enabled: false },
        onConnectionError: (event) => connectionErrors.push(event.attempt),
        onChange: async () => {},
      }),
    /server-unavailable/,
  );

  assert.equal(runs, 1);
  // onConnectionError fires even when retries are disabled
  assert.deepEqual(connectionErrors, [1]);
});

// ---------------------------------------------------------------------------
// Scenario 7 — connectionRetry.maxAttempts exhausted
// ---------------------------------------------------------------------------
test('connectionRetry maxAttempts exhausted fires onConnectionError and rethrows', async () => {
  let runs = 0;
  const client = {
    query: async () => ({ status: 'success', results: [] }),
    queryOne: async () => null,
    queryAll: async () => [],
    consumer: () => ({
      run: async () => {
        runs += 1;
        throw new Error('server-down');
      },
      stop: () => {},
    }),
  };

  const retries = [];
  const connectionErrors = [];

  await assert.rejects(
    () =>
      runConsumer({
        client,
        name: 'budget-exhausted',
        topic: 'orders',
        groupId: 'billing-worker',
        connectionRetry: {
          maxAttempts: 2, // 1 initial + 1 retry = 2 total run attempts
          initialBackoffMs: 0,
          maxBackoffMs: 0,
          jitterRatio: 0,
        },
        onConnectionRetry: (event) => retries.push(event.attempt),
        onConnectionError: (event) => connectionErrors.push(event.attempt),
        onChange: async () => {},
      }),
    /server-down/,
  );

  // With maxAttempts=2: run 1 fails (attempt 1, retries [1]), run 2 fails (attempt 2, budget exhausted)
  assert.equal(runs, 2);
  assert.deepEqual(retries, [1]);
  assert.deepEqual(connectionErrors, [2]);
});

// ---------------------------------------------------------------------------
// Scenario 8 — Retry counter resets to 1 for each new message
// ---------------------------------------------------------------------------
test('retry attempt counter resets to 1 for each new message in sequence', async () => {
  const messages = [
    makeMessage({ offset: 208 }),
    makeMessage({ offset: 209 }),
    makeMessage({ offset: 210 }),
  ];
  const { client, state } = createMockClient(messages);

  const attemptLog = []; // { offset, attempt }
  let msg209calls = 0;

  await runConsumer({
    client,
    name: 'retry-isolation',
    topic: messages[0].topic,
    groupId: messages[0].group_id,
    retry: { maxAttempts: 3, initialBackoffMs: 0, maxBackoffMs: 0 },
    onChange: async (ctx, change) => {
      attemptLog.push({ offset: change.offset, attempt: ctx.attempt });
      // Message 209 is transiently unstable — fails first two attempts
      if (change.offset === 209) {
        msg209calls += 1;
        if (msg209calls < 3) {
          throw new Error('transient-error');
        }
      }
    },
  });

  assert.deepEqual(state.ackedOffsets, [208, 209, 210]);

  // Message 208: single attempt, clean success
  assert.deepEqual(
    attemptLog.filter((e) => e.offset === 208),
    [{ offset: 208, attempt: 1 }],
  );

  // Message 209: three attempts, succeeds on third
  assert.deepEqual(
    attemptLog.filter((e) => e.offset === 209),
    [
      { offset: 209, attempt: 1 },
      { offset: 209, attempt: 2 },
      { offset: 209, attempt: 3 },
    ],
  );

  // Message 210: fresh counter starting at 1 — no bleed from message 209
  assert.deepEqual(
    attemptLog.filter((e) => e.offset === 210),
    [{ offset: 210, attempt: 1 }],
  );
});

// ---------------------------------------------------------------------------
// Scenario 9 — Pre-aborted stopSignal exits immediately without any work
// ---------------------------------------------------------------------------
test('pre-aborted stopSignal exits immediately without processing messages', async () => {
  const message = makeMessage({ offset: 211 });
  const { client, state } = createMockClient([message]);

  let calls = 0;
  const controller = new AbortController();
  controller.abort(); // signal already aborted before runConsumer is called

  await runConsumer({
    client,
    name: 'pre-aborted-worker',
    topic: message.topic,
    groupId: message.group_id,
    stopSignal: controller.signal,
    retry: { maxAttempts: 1, initialBackoffMs: 0, maxBackoffMs: 0 },
    onChange: async () => {
      calls += 1;
    },
  });

  // The outer while loop checks stopSignal?.aborted before entering —
  // it must exit immediately without creating a consumer handle or
  // dispatching any messages.
  assert.equal(calls, 0);
  assert.deepEqual(state.ackedOffsets, []);
  // consumer() must never have been called if the loop never ran
  assert.equal(state.consumerOptions.length, 0);
});

// ---------------------------------------------------------------------------
// Scenario 10 — Custom runKeyFactory propagates key through all event handlers
// ---------------------------------------------------------------------------
test('custom runKeyFactory key propagates to ctx.runKey onRetry and onFailed', async () => {
  const message = makeMessage({ offset: 220 });
  const { client, state } = createMockClient([message]);

  const keys = { ctx: [], retries: [], failed: null, error: null };

  await runConsumer({
    client,
    name: 'keyfactory-worker',
    topic: message.topic,
    groupId: message.group_id,
    retry: { maxAttempts: 3, initialBackoffMs: 0, maxBackoffMs: 0 },
    // Production pattern: include deployment-specific prefix for global
    // idempotency when multiple worker fleets share the same topic.
    runKeyFactory: ({ name, message: msg }) =>
      `prod:${name}:${msg.topic}:${msg.partition_id}:${msg.offset}`,
    onChange: async (ctx) => {
      keys.ctx.push(ctx.runKey);
      throw new Error('always-fail');
    },
    onRetry: (event) => {
      keys.retries.push(event.runKey);
    },
    onError: (event) => {
      keys.error = String(event.error);
    },
    onFailed: async (ctx) => {
      keys.failed = ctx.runKey;
    },
    ackOnFailed: true,
  });

  const expectedKey = `prod:keyfactory-worker:${message.topic}:${message.partition_id}:${message.offset}`;

  // runKey visible in every onChange invocation
  assert.deepEqual(keys.ctx, [expectedKey, expectedKey, expectedKey]);
  // runKey in onRetry events (fires after attempts 1 and 2 — not after the last)
  assert.deepEqual(keys.retries, [expectedKey, expectedKey]);
  // runKey in onFailed context after all retries exhausted
  assert.equal(keys.failed, expectedKey);
  assert.deepEqual(state.ackedOffsets, [220]);
});

// ---------------------------------------------------------------------------
// Scenario 11 — onConnectionRestored fires via onBatchSuccess on empty poll
// ---------------------------------------------------------------------------
// This scenario specifically exercises the onBatchSuccess hook path. In
// production, after a server restart the first successful poll is often an
// empty batch (the cursor is already at the head), so onConnectionRestored
// must fire without waiting for an actual message.
test('onConnectionRestored fires via empty-poll onBatchSuccess path after reconnect', async () => {
  let runCount = 0;
  const restored = [];
  const retries = [];

  const client = {
    query: async () => ({ status: 'success', results: [] }),
    queryOne: async () => null,
    queryAll: async () => [],
    consumer: () => ({
      // run() accepts the hooks second argument and uses it
      run: async (_handler, hooks) => {
        runCount += 1;
        if (runCount === 1) {
          // Simulate server going down before returning any messages
          throw new Error('server-restarting');
        }
        // Run 2: server is back but there are no new messages yet.
        // onBatchSuccess fires (empty batch) — this must trigger onConnectionRestored.
        hooks?.onBatchSuccess?.({ nextOffset: 0, hasMore: false, messageCount: 0 });
        // No message dispatch — the consumer loop returns cleanly.
      },
      stop: () => {},
    }),
  };

  let calls = 0;
  await runConsumer({
    client,
    name: 'empty-poll-restore',
    topic: 'notifications',
    groupId: 'notification-worker',
    connectionRetry: { initialBackoffMs: 0, maxBackoffMs: 0, jitterRatio: 0 },
    onConnectionRetry: (event) => retries.push(event.attempt),
    onConnectionRestored: (event) => restored.push(event.attempt),
    onChange: async () => {
      calls += 1;
    },
  });

  assert.equal(runCount, 2);
  // No messages were processed — restoration confirmed by empty poll alone
  assert.equal(calls, 0);
  // Retried once before recovery
  assert.deepEqual(retries, [1]);
  // onConnectionRestored fired via the onBatchSuccess hook (empty poll path)
  assert.deepEqual(restored, [1]);
});
