import assert from 'node:assert/strict';
import test from 'node:test';

import {
  createLangChainAdapter,
  runAgent,
  runConsumer,
} from '../dist/src/index.js';

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

function createMockClient(messages, options = {}) {
  const state = {
    consumerOptions: [],
    ackedOffsets: [],
    queryCalls: [],
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
        run: async (handler) => {
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

          if (options.runMode === 'parallel') {
            const pending = [];
            for (const message of messages) {
              if (stopped) {
                break;
              }
              pending.push(dispatchMessage(message));
            }
            await Promise.all(pending);
            return;
          }

          for (const message of messages) {
            if (stopped) {
              break;
            }

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

test('runConsumer retries and acks once after success', async () => {
  const message = makeMessage({ user: 'alice', change: { data: 'raw transport change' } });
  const { client, state } = createMockClient([message]);

  let attempts = 0;
  const retries = [];
  const seenUsers = [];

  await runConsumer({
    client,
    name: 'summarizer-agent',
    topic: message.topic,
    groupId: message.group_id,
    retry: {
      maxAttempts: 3,
      initialBackoffMs: 0,
      maxBackoffMs: 0,
    },
    onRetry: (event) => retries.push(event.attempt),
    onChange: async (ctx, change) => {
      attempts += 1;
      seenUsers.push(change.user);
      assert.equal(change.op, undefined);
      assert.equal(change.message.user, 'alice');
      assert.equal('row' in ctx, false);
      assert.equal('message' in ctx, false);
      assert.equal('change' in ctx, false);
      assert.equal('user' in ctx, false);
      assert.equal('op' in ctx, false);
      assert.equal('offset' in ctx, false);
      assert.equal('payload' in change.message, false);
      assert.equal('value' in change.message, false);
      assert.equal('change' in change.message, false);
      assert.equal(change.data.content, 'hello world');
      assert.deepEqual(Object.keys(change), [
        'data',
        'message',
        'user',
        'key',
        'op',
        'timestampMs',
        'partitionId',
        'offset',
        'topic',
        'groupId',
      ]);
      if (attempts < 3) {
        throw new Error('transient failure');
      }
    },
  });

  assert.equal(attempts, 3);
  assert.deepEqual(retries, [1, 2]);
  assert.deepEqual(seenUsers, ['alice', 'alice', 'alice']);
  assert.deepEqual(state.ackedOffsets, [7]);
  assert.equal(state.consumerOptions[0].auto_ack, false);
});

test('runConsumer exposes the user in context when it is received', async () => {
  const message = makeMessage({
    offset: 51,
    user: 'alice',
    payload: { blog_id: '51', content: 'hello user', _table: 'blog.posts' },
    value: { blog_id: '51', content: 'hello user', _table: 'blog.posts' },
  });
  const { client, state } = createMockClient([message]);

  const seenUsers = [];

  await runConsumer({
    client,
    name: 'user-context-check',
    topic: message.topic,
    groupId: message.group_id,
    retry: {
      maxAttempts: 1,
      initialBackoffMs: 0,
      maxBackoffMs: 0,
    },
    onChange: async (ctx, change) => {
      seenUsers.push({
        changeUser: change.user,
        messageUser: change.message.user,
        hasCtxUser: 'user' in ctx,
        blogId: change.data.blog_id,
      });
    },
  });

  assert.deepEqual(seenUsers, [{ changeUser: 'alice', messageUser: 'alice', hasCtxUser: false, blogId: '51' }]);
  assert.deepEqual(state.ackedOffsets, [51]);
});

test('runConsumer exposes the op in context when it is received', async () => {
  const message = makeMessage({
    offset: 53,
    op: 'Update',
    payload: { blog_id: '53', content: 'updated row', _table: 'blog.posts' },
    value: { blog_id: '53', content: 'updated row', _table: 'blog.posts' },
  });
  const { client, state } = createMockClient([message]);

  const ops = [];

  await runConsumer({
    client,
    name: 'op-context-check',
    topic: message.topic,
    groupId: message.group_id,
    retry: {
      maxAttempts: 1,
      initialBackoffMs: 0,
      maxBackoffMs: 0,
    },
    onChange: async (ctx, change) => {
      ops.push({
        op: change.op,
        offset: change.offset,
        hasCtxOp: 'op' in ctx,
        hasCtxOffset: 'offset' in ctx,
        blogId: change.data.blog_id,
      });
    },
  });

  assert.deepEqual(ops, [{ op: 'Update', offset: 53, hasCtxOp: false, hasCtxOffset: false, blogId: '53' }]);
  assert.deepEqual(state.ackedOffsets, [53]);
});

test('runConsumer preserves seqid and system fields inside change.data', async () => {
  const message = makeMessage({
    offset: 55,
    payload: {
      blog_id: '55',
      content: 'system fields intact',
      _table: 'blog.posts',
      _row_id: 'row-55',
      _seqid: '9001',
      _version: 3,
    },
    value: {
      blog_id: '55',
      content: 'system fields intact',
      _table: 'blog.posts',
      _row_id: 'row-55',
      _seqid: '9001',
      _version: 3,
    },
  });
  const { client, state } = createMockClient([message]);

  let seenChange = null;

  await runConsumer({
    client,
    name: 'seqid-system-fields-check',
    topic: message.topic,
    groupId: message.group_id,
    retry: {
      maxAttempts: 1,
      initialBackoffMs: 0,
      maxBackoffMs: 0,
    },
    onChange: async (_ctx, change) => {
      seenChange = change.data;
    },
  });

  assert.deepEqual(seenChange, {
    blog_id: '55',
    content: 'system fields intact',
    _table: 'blog.posts',
    _row_id: 'row-55',
    _seqid: '9001',
    _version: 3,
  });
  assert.deepEqual(state.ackedOffsets, [55]);
});

test('runConsumer keeps mixed insert update and delete contexts isolated in one run', async () => {
  const messages = [
    makeMessage({
      offset: 61,
      user: 'alice',
      op: 'Insert',
      payload: { blog_id: '61', content: 'created', _table: 'blog.posts', _seqid: '100' },
      value: { blog_id: '61', content: 'created', _table: 'blog.posts', _seqid: '100' },
    }),
    makeMessage({
      offset: 62,
      user: 'bob',
      op: 'Update',
      payload: { blog_id: '61', content: 'updated', summary: 'done', _table: 'blog.posts', _seqid: '101' },
      value: { blog_id: '61', content: 'updated', summary: 'done', _table: 'blog.posts', _seqid: '101' },
    }),
    makeMessage({
      offset: 63,
      user: 'carol',
      op: 'Delete',
      payload: { blog_id: '61', _table: 'blog.posts', _seqid: '102', _deleted: true },
      value: { blog_id: '61', _table: 'blog.posts', _seqid: '102', _deleted: true },
    }),
  ];
  const { client, state } = createMockClient(messages);

  const seenContexts = [];

  await runConsumer({
    client,
    name: 'mixed-change-context-check',
    topic: messages[0].topic,
    groupId: messages[0].group_id,
    retry: {
      maxAttempts: 1,
      initialBackoffMs: 0,
      maxBackoffMs: 0,
    },
    onChange: async (ctx, change) => {
      seenContexts.push({
        offset: change.offset,
        user: change.user,
        op: change.op,
        topic: change.topic,
        groupId: change.groupId,
        seqid: change.data._seqid,
        blogId: change.data.blog_id,
        content: change.data.content ?? null,
        deleted: change.data._deleted ?? false,
        ctxHasMessage: 'message' in ctx,
      });
    },
  });

  assert.deepEqual(seenContexts, [
    {
      offset: 61,
      user: 'alice',
      op: 'Insert',
      topic: 'blog.summarizer',
      groupId: 'blog-summarizer-agent',
      seqid: '100',
      blogId: '61',
      content: 'created',
      deleted: false,
      ctxHasMessage: false,
    },
    {
      offset: 62,
      user: 'bob',
      op: 'Update',
      topic: 'blog.summarizer',
      groupId: 'blog-summarizer-agent',
      seqid: '101',
      blogId: '61',
      content: 'updated',
      deleted: false,
      ctxHasMessage: false,
    },
    {
      offset: 63,
      user: 'carol',
      op: 'Delete',
      topic: 'blog.summarizer',
      groupId: 'blog-summarizer-agent',
      seqid: '102',
      blogId: '61',
      content: null,
      deleted: true,
      ctxHasMessage: false,
    },
  ]);
  assert.deepEqual(state.ackedOffsets, [61, 62, 63]);
});

test('runConsumer keeps concurrent handler contexts isolated', async () => {
  const messages = [
    makeMessage({
      offset: 71,
      user: 'alice',
      op: 'Insert',
      payload: { blog_id: '71', content: 'parallel insert', _table: 'blog.posts', _seqid: '201' },
      value: { blog_id: '71', content: 'parallel insert', _table: 'blog.posts', _seqid: '201' },
    }),
    makeMessage({
      offset: 72,
      user: 'bob',
      op: 'Update',
      payload: { blog_id: '72', content: 'parallel update', _table: 'blog.posts', _seqid: '202' },
      value: { blog_id: '72', content: 'parallel update', _table: 'blog.posts', _seqid: '202' },
    }),
    makeMessage({
      offset: 73,
      user: 'carol',
      op: 'Delete',
      payload: { blog_id: '73', _table: 'blog.posts', _seqid: '203', _deleted: true },
      value: { blog_id: '73', _table: 'blog.posts', _seqid: '203', _deleted: true },
    }),
  ];
  const { client, state } = createMockClient(messages, { runMode: 'parallel' });

  const contexts = [];
  const changes = [];
  const seen = [];

  await runConsumer({
    client,
    name: 'parallel-context-check',
    topic: messages[0].topic,
    groupId: messages[0].group_id,
    retry: {
      maxAttempts: 1,
      initialBackoffMs: 0,
      maxBackoffMs: 0,
    },
    onChange: async (ctx, change) => {
      contexts.push(ctx);
      changes.push(change);
      await new Promise((resolve) => setTimeout(resolve, 74 - change.offset));
      seen.push({
        offset: change.offset,
        user: change.user,
        op: change.op,
        seqid: change.data._seqid,
        blogId: change.data.blog_id,
      });
    },
  });

  const sortedSeen = [...seen].sort((left, right) => left.offset - right.offset);
  const sortedAcked = [...state.ackedOffsets].sort((left, right) => left - right);

  assert.equal(new Set(contexts).size, 3);
  assert.equal(new Set(changes).size, 3);
  assert.deepEqual(sortedSeen, [
    { offset: 71, user: 'alice', op: 'Insert', seqid: '201', blogId: '71' },
    { offset: 72, user: 'bob', op: 'Update', seqid: '202', blogId: '72' },
    { offset: 73, user: 'carol', op: 'Delete', seqid: '203', blogId: '73' },
  ]);
  assert.deepEqual(sortedAcked, [71, 72, 73]);
});

test('runConsumer calls onFailed and then acks when configured', async () => {
  const message = makeMessage({ offset: 9 });
  const { client, state } = createMockClient([message]);

  let onFailedCalls = 0;
  let failedRunKey = '';

  await runConsumer({
    client,
    name: 'summarizer-agent',
    topic: message.topic,
    groupId: message.group_id,
    retry: {
      maxAttempts: 2,
      initialBackoffMs: 0,
      maxBackoffMs: 0,
    },
    onChange: async () => {
      throw new Error('permanent failure');
    },
    onFailed: async (ctx, change) => {
      onFailedCalls += 1;
      failedRunKey = ctx.runKey;
      assert.equal(change.data.blog_id, '42');
      assert.equal('change' in ctx, false);
      await ctx.sql('INSERT INTO blog.summary_failures VALUES ($1)', [ctx.runKey]);
    },
    ackOnFailed: true,
  });

  assert.equal(onFailedCalls, 1);
  assert.match(failedRunKey, /^summarizer-agent:/);
  assert.deepEqual(state.ackedOffsets, [9]);
  assert.equal(state.queryCalls.length, 1);
});

test('runConsumer does not ack when onFailed throws', async () => {
  const message = makeMessage({ offset: 11 });
  const { client, state } = createMockClient([message]);

  const errors = [];

  await runConsumer({
    client,
    name: 'summarizer-agent',
    topic: message.topic,
    groupId: message.group_id,
    retry: {
      maxAttempts: 1,
      initialBackoffMs: 0,
      maxBackoffMs: 0,
    },
    onChange: async () => {
      throw new Error('always fail');
    },
    onFailed: async () => {
      throw new Error('failed sink write');
    },
    onError: (event) => {
      errors.push(String(event.error));
    },
  });

  assert.deepEqual(state.ackedOffsets, []);
  assert.equal(errors.length, 1);
  assert.match(errors[0], /failed sink write/);
});

test('runConsumer exposes llm context with system prompt metadata', async () => {
  const message = makeMessage({
    offset: 13,
    payload: { blog_id: '13', content: 'A long blog body', _table: 'blog.posts' },
    value: { blog_id: '13', content: 'A long blog body', _table: 'blog.posts' },
  });
  const { client, state } = createMockClient([message]);

  const llmInputs = [];

  await runConsumer({
    client,
    name: 'summarizer-agent',
    topic: message.topic,
    groupId: message.group_id,
    systemPrompt: 'system prompt',
    llm: {
      complete: async (input) => {
        llmInputs.push(input);
        return 'summary';
      },
    },
    onChange: async (ctx) => {
      const result = await ctx.llm.complete('summarize');
      assert.equal(result, 'summary');
      assert.ok(ctx.runKey.includes(':13'));
    },
  });

  assert.equal(llmInputs.length, 1);
  assert.equal(llmInputs[0].systemPrompt, 'system prompt');
  assert.equal(llmInputs[0].prompt, 'summarize');
  assert.deepEqual(state.ackedOffsets, [13]);
});

test('runConsumer still unwraps legacy payload.row envelopes', async () => {
  const message = makeMessage({
    payload: { row: { blog_id: '52', content: 'wrapped payload' } },
    value: { row: { blog_id: '52', content: 'wrapped payload' } },
  });
  const { client, state } = createMockClient([message]);

  let seenRow = null;

  await runConsumer({
    client,
    name: 'summarizer-agent',
    topic: message.topic,
    groupId: message.group_id,
    retry: {
      maxAttempts: 1,
      initialBackoffMs: 0,
      maxBackoffMs: 0,
    },
    onChange: async (_ctx, change) => {
      seenRow = change.data;
    },
  });

  assert.deepEqual(seenRow, { blog_id: '52', content: 'wrapped payload' });
  assert.deepEqual(state.ackedOffsets, [7]);
});

test('runConsumer supports the deprecated onMessage handler', async () => {
  const message = makeMessage({ offset: 21 });
  const { client, state } = createMockClient([message]);

  let calls = 0;

  await runConsumer({
    client,
    name: 'consumer-runtime',
    topic: message.topic,
    groupId: message.group_id,
    retry: {
      maxAttempts: 1,
      initialBackoffMs: 0,
      maxBackoffMs: 0,
    },
    onMessage: async (ctx, change) => {
      calls += 1;
      assert.equal('message' in ctx, false);
      assert.equal(change.offset, 21);
      assert.equal('payload' in change.message, false);
      assert.equal('value' in change.message, false);
      assert.equal(change.data.content, 'hello world');
    },
  });

  assert.equal(calls, 1);
  assert.deepEqual(state.ackedOffsets, [21]);
});

test('runConsumer reconnects after transient consumer loop errors', async () => {
  const message = makeMessage({ offset: 31 });
  const state = {
    runs: 0,
    ackedOffsets: [],
  };

  const client = {
    query: async () => ({ status: 'success', results: [] }),
    queryOne: async () => null,
    queryAll: async () => [],
    consumer: () => ({
      run: async (handler) => {
        state.runs += 1;
        if (state.runs < 3) {
          throw new Error(`disconnect-${state.runs}`);
        }
        await handler({
          user: message.user,
          message,
          ack: async () => {
            state.ackedOffsets.push(message.offset);
          },
        });
      },
      stop: () => {},
    }),
  };

  const retries = [];
  let calls = 0;

  await runConsumer({
    client,
    name: 'summarizer-agent',
    topic: message.topic,
    groupId: message.group_id,
    retry: {
      maxAttempts: 1,
      initialBackoffMs: 0,
      maxBackoffMs: 0,
    },
    connectionRetry: {
      initialBackoffMs: 0,
      maxBackoffMs: 0,
      jitterRatio: 0,
    },
    onConnectionRetry: (event) => retries.push(event.attempt),
    onChange: async () => {
      calls += 1;
    },
  });

  assert.equal(state.runs, 3);
  assert.equal(calls, 1);
  assert.deepEqual(retries, [1, 2]);
  assert.deepEqual(state.ackedOffsets, [31]);
});

test('runConsumer reconnects cleanly when the server goes down after processing starts', async () => {
  const firstMessage = makeMessage({
    offset: 81,
    user: 'alice',
    op: 'Insert',
    payload: { blog_id: '81', content: 'before disconnect', _table: 'blog.posts', _seqid: '301' },
    value: { blog_id: '81', content: 'before disconnect', _table: 'blog.posts', _seqid: '301' },
  });
  const secondMessage = makeMessage({
    offset: 82,
    user: 'bob',
    op: 'Update',
    payload: { blog_id: '82', content: 'after reconnect', _table: 'blog.posts', _seqid: '302' },
    value: { blog_id: '82', content: 'after reconnect', _table: 'blog.posts', _seqid: '302' },
  });

  const state = {
    runs: 0,
    ackedOffsets: [],
  };

  const client = {
    query: async () => ({ status: 'success', results: [] }),
    queryOne: async () => null,
    queryAll: async () => [],
    consumer: () => ({
      run: async (handler) => {
        state.runs += 1;

        if (state.runs === 1) {
          await handler({
            user: firstMessage.user,
            message: firstMessage,
            ack: async () => {
              state.ackedOffsets.push(firstMessage.offset);
            },
          });
          throw new Error('server went down');
        }

        await handler({
          user: secondMessage.user,
          message: secondMessage,
          ack: async () => {
            state.ackedOffsets.push(secondMessage.offset);
          },
        });
      },
      stop: () => {},
    }),
  };

  const retries = [];
  const seen = [];

  await runConsumer({
    client,
    name: 'reconnect-after-start',
    topic: firstMessage.topic,
    groupId: firstMessage.group_id,
    retry: {
      maxAttempts: 1,
      initialBackoffMs: 0,
      maxBackoffMs: 0,
    },
    connectionRetry: {
      initialBackoffMs: 0,
      maxBackoffMs: 0,
      jitterRatio: 0,
    },
    onConnectionRetry: (event) => retries.push(event.attempt),
    onChange: async (_ctx, change) => {
      seen.push({
        offset: change.offset,
        user: change.user,
        op: change.op,
        seqid: change.data._seqid,
        content: change.data.content,
      });
    },
  });

  assert.equal(state.runs, 2);
  assert.deepEqual(retries, [1]);
  assert.deepEqual(seen, [
    { offset: 81, user: 'alice', op: 'Insert', seqid: '301', content: 'before disconnect' },
    { offset: 82, user: 'bob', op: 'Update', seqid: '302', content: 'after reconnect' },
  ]);
  assert.deepEqual(state.ackedOffsets, [81, 82]);
});

test('runConsumer falls back to message.user when a custom client omits ctx.user', async () => {
  const message = makeMessage({
    offset: 91,
    user: 'root',
    payload: { blog_id: '91', content: 'shared row', _table: 'blog.posts' },
    value: { blog_id: '91', content: 'shared row', _table: 'blog.posts' },
  });

  const client = {
    query: async () => ({ status: 'success', results: [] }),
    queryOne: async () => null,
    queryAll: async () => [],
    consumer: () => ({
      run: async (handler) => {
        await handler({
          user: undefined,
          message,
          ack: async () => {},
        });
      },
      stop: () => {},
    }),
  };

  let seen;
  await runConsumer({
    client,
    name: 'message-user-fallback',
    topic: message.topic,
    groupId: message.group_id,
    retry: {
      maxAttempts: 1,
      initialBackoffMs: 0,
      maxBackoffMs: 0,
    },
    onChange: async (_ctx, change) => {
      seen = {
        changeUser: change.user,
        messageUser: change.message.user,
      };
    },
  });

  assert.deepEqual(seen, {
    changeUser: 'root',
    messageUser: 'root',
  });
});

test('runAgent remains a compatibility alias for runConsumer', async () => {
  const message = makeMessage({ offset: 41 });
  const { client, state } = createMockClient([message]);
  let calls = 0;

  await runAgent({
    client,
    name: 'legacy-agent-name',
    topic: message.topic,
    groupId: message.group_id,
    retry: {
      maxAttempts: 1,
      initialBackoffMs: 0,
      maxBackoffMs: 0,
    },
    onChange: async (_ctx, change) => {
      calls += 1;
      assert.equal(change.data.content, 'hello world');
    },
  });

  assert.equal(calls, 1);
  assert.deepEqual(state.ackedOffsets, [41]);
});

test('createLangChainAdapter normalizes completion and stream outputs', async () => {
  const invokeInputs = [];
  const streamInputs = [];

  const adapter = createLangChainAdapter({
    invoke: async (input) => {
      invokeInputs.push(input);
      return { content: [{ text: 'abc' }] };
    },
    stream: async function* (input) {
      streamInputs.push(input);
      yield { content: [{ text: 'x' }] };
      yield { content: [{ text: 'y' }] };
    },
  });

  const completed = await adapter.complete({
    systemPrompt: 'sys',
    prompt: 'hello',
  });
  assert.equal(completed, 'abc');

  const streamed = [];
  for await (const chunk of adapter.stream({ prompt: 'hello' })) {
    streamed.push(chunk);
  }
  assert.deepEqual(streamed, ['x', 'y']);

  assert.equal(invokeInputs.length, 1);
  assert.equal(streamInputs.length, 1);
});