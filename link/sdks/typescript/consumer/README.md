# @kalamdb/consumer

Topic consumer worker runtime package for KalamDB.

Use `@kalamdb/client` for app-facing SQL, live rows, subscriptions, and file uploads. Add `@kalamdb/consumer` only when you need topic polling, acknowledgments, or the high-level worker runtime.

`@kalamdb/consumer` ships its own worker-focused WASM bundle and layers it on top of `@kalamdb/client`, but that worker bundle is intentionally limited to topic consume and ack transport instead of re-shipping the main client WASM surface. App-only installs can keep using the lighter main client package alone.

> Status: **Beta**.

## Installation

```bash
npm i @kalamdb/client @kalamdb/consumer
```

## What This Package Owns

- `consumeBatch()` for one-shot topic polling
- `ack()` for explicit offset commits
- `consumer().run()` for continuous polling loops
- `runConsumer()` for higher-level worker orchestration with retries, ACKs, and reconnects
- `runAgent()` as a deprecated compatibility alias for `runConsumer()`

Topic HTTP endpoints require bearer authentication and role `service`, `dba`, or `system`.

## Quick Start

```ts
import { Auth } from '@kalamdb/client';
import { createConsumerClient, runConsumer } from '@kalamdb/consumer';

const client = createConsumerClient({
  url: 'http://localhost:8080',
  authProvider: async () => Auth.basic('support-worker', 'Secret123!'),
});

await runConsumer({
  client,
  name: 'support-summary-agent',
  topic: 'support.inbox_events',
  groupId: 'support-summary-agent',
  retry: {
    maxAttempts: 3,
    initialBackoffMs: 250,
    maxBackoffMs: 2_000,
  },
  onChange: async (_ctx, change) => {
    const user = String(change.user).trim();
    const row = change.data;
    const body = String(row.body ?? '').trim();
    if (!user || !body) {
      return;
    }

    const summary = `Support summary: ${body.slice(0, 120)}`;
    await client.executeAsUser(
      'INSERT INTO support.inbox (room, role, body) VALUES ($1, $2, $3)',
      user,
      ['main', 'assistant', summary],
    );
  },
});
```

For standard KalamDB topic sources, `runConsumer()` does not need a parser. The runtime uses the already decoded low-level `message.payload`, unwraps legacy `{ row: ... }` envelopes when present, and exposes the changed row/event as `change.data`. Per-change envelope metadata also lives on `change`: `user`, typed `op`, `key`, `timestampMs`, `partitionId`, `offset`, `topic`, `groupId`, and a metadata-only `message` view. The high-level runtime intentionally keeps `ctx` for execution state and helpers only: `name`, `runKey`, retry attempt fields, SQL helpers, ACK, and optional LLM helpers. That means high-level `ctx` has no `message`, `change`, `user`, `op`, or `offset` duplicates. `change.message` intentionally omits `payload`, deprecated `value`, and raw transport `change` fields, so the row shape lives in one place: `change.data`. `change.user` is required for consumed topic events; if a server or republished topic message omits it, the consumer treats that as invalid message metadata instead of exposing `undefined`. Add `changeParser` only when you intentionally publish a custom payload shape.

`runConsumer()` keeps the worker alive across transient server shutdowns or network disconnects by retrying the consumer loop with exponential backoff and jitter. Tune this with `connectionRetry` or stop cleanly with `stopSignal`.

## Lower-Level Consumer

```ts
import { Auth } from '@kalamdb/client';
import { createConsumerClient } from '@kalamdb/consumer';

const client = createConsumerClient({
  url: 'http://localhost:8080',
  authProvider: async () => Auth.jwt(await getWorkerToken()),
});

const handle = client.consumer({
  topic: 'support.inbox_events',
  group_id: 'support-worker',
  auto_ack: true,
  batch_size: 10,
});

await handle.run(async (ctx) => {
  console.log(
    ctx.message.topic,
    ctx.message.partition_id,
    ctx.message.offset,
    ctx.message.op,
    ctx.message.user,
    ctx.message.payload,
  );
});
```

## One-Shot Polling

```ts
const batch = await client.consumeBatch({
  topic: 'support.inbox_events',
  group_id: 'support-worker',
  start: 'earliest',
  batch_size: 25,
});

for (const message of batch.messages) {
  console.log(message.offset, message.key, message.timestamp_ms, message.payload);
}

if (batch.messages.length > 0) {
  const last = batch.messages[batch.messages.length - 1];
  await client.ack(last.topic, last.group_id, last.partition_id, last.offset);
}
```

## Message Shape

Each consumed message includes the current backend topic envelope fields:

```ts
{
  topic: 'support.inbox_events',
  group_id: 'support-worker',
  partition_id: 0,
  offset: 42,
  key: '{"id":"01HS..."}',
  timestamp_ms: 1730000000000,
  user: 'user_123',
  op: 'Insert', // TopicOp.Insert
  payload: {
    id: '01HS...',
    author: 'user',
    body: 'Please summarize this support thread',
    _table: 'support.inbox',
  },
}
```

If you know your payload shape, you can type the whole consumer flow directly:

```ts
type SupportInboxPayload = {
  id: string;
  author: string;
  body: string;
  _table: string;
};

const batch = await client.consumeBatch<SupportInboxPayload>({
  topic: 'support.inbox_events',
  group_id: 'support-worker',
  start: 'earliest',
});

for (const message of batch.messages) {
  console.log(message.payload.body);
}

const handle = client.consumer<SupportInboxPayload>({
  topic: 'support.inbox_events',
  group_id: 'support-worker',
});

await handle.run(async (ctx) => {
  console.log(ctx.message.payload.body);
});
```

The same `change` shape works with generated ORM row types. Type the row as the first generic, use `change.data` for the row, and keep event metadata next to it on `change`:

```ts
type BlogRow = {
  blog_id: string;
  title: string;
  content: string;
  _table: string;
  _seqid?: string;
};

await runConsumer<BlogRow>({
  client,
  name: 'blog-worker',
  topic: 'blog.events',
  groupId: 'blog-worker',
  onChange: async (ctx, change) => {
    const row = change.data;
    console.log(change.op, change.user, row.blog_id, row._seqid);

    await ctx.sql(
      'UPDATE blog.blogs SET updated = NOW() WHERE blog_id = $1',
      [row.blog_id],
    );
  },
});
```

Notes:

- `payload` is already decoded from the HTTP API's base64 `payload` field.
- For `WITH (payload = 'full')`, `payload` is usually the changed row JSON plus `_table` metadata.
- `runConsumer()` automatically treats that decoded row payload as `change.data`.
- `op` is typed as `TopicOp` (`'Insert' | 'Update' | 'Delete'`).
- `value` is still present as a deprecated alias for `payload` while older callers migrate.
- `key` is the backend topic key string. It is not a separate message id.

## Notes

- `Auth.basic(user, password)` is exchanged on `POST /v1/api/auth/login` before topic requests.
- Topic payloads are decoded from the HTTP API's base64 payload field and exposed as `message.payload`.
- When you only need browser/app features, install `@kalamdb/client` alone.
- Low-level worker bindings are also available at `@kalamdb/consumer/wasm`.

## License

Licensed under the Apache License, Version 2.0 (`Apache-2.0`). See the packaged `LICENSE.txt` and `NOTICE` files.