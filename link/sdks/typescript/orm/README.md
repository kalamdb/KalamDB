# @kalamdb/orm

Drizzle ORM driver, KalamDB table helpers, live table helpers, and schema generator for KalamDB.

Use this package with `@kalamdb/client` when you want Drizzle-style queries in a browser app, admin UI, or Node service. Use `@kalamdb/consumer` separately for topic workers and agents. Use the same generated table definitions with `@kalamdb/react` when a React UI needs typed live-query hooks and component wrappers.

## Install

```bash
npm i @kalamdb/client @kalamdb/orm drizzle-orm
```

## Driver quick start

```ts
import { Auth, createClient } from '@kalamdb/client';
import { kalamDriver } from '@kalamdb/orm';
import { drizzle } from 'drizzle-orm/pg-proxy';
import { messages } from './schema';

const client = createClient({
  url: 'http://localhost:2900',
  namespace: 'app',
  authProvider: async () => Auth.basic('admin', 'AdminPass123!'),
});

const db = drizzle(kalamDriver(client));
const rows = await db.select().from(messages).limit(20);
```

The driver normalizes KalamDB temporal wire values for Drizzle columns: `timestamp(..., { mode: 'date' })`, `date(..., { mode: 'date' })`, and `time(...)` are converted from the numeric wire representation when needed.

When your client works inside one default namespace, set `createClient({ namespace: 'app' })` so unqualified ORM queries and `liveTable()` subscriptions resolve through that namespace automatically. Keep `configureKalamOrm({ namespace: 'app' })` for generated or hand-written unqualified table definitions so the Drizzle metadata matches the same scope.

## Table helpers

```ts
import { bytes, embedding, file, kTable } from '@kalamdb/orm';
import { bigint, jsonb, text, timestamp, uuid } from 'drizzle-orm/pg-core';

export const docs = kTable.shared('app.docs', {
  id: bigint('id', { mode: 'bigint' }).primaryKey(),
  owner_id: uuid('owner_id').notNull(),
  title: text('title').notNull(),
  body: text('body'),
  metadata: jsonb('metadata'),
  attachment: file('attachment'),
  raw_bytes: bytes('raw_bytes'),
  embedding: embedding('embedding', 384),
  created_at: timestamp('created_at', { mode: 'date' }).notNull(),
});
```

`file()`, `bytes()`, and `embedding()` are KalamDB-specific Drizzle custom columns. `file()` maps values to `FileRef | null`, `bytes()` maps to `Uint8Array | null`, and `embedding()` maps to `number[] | null`.

Use the table-kind helpers when the table kind matters to live views or agents:

```ts
kTable.shared('app.docs', columns);
kTable.user('app.messages', columns);
kTable.stream('app.events', columns);
kTable.system('system.users', columns);
```

When you want Drizzle-style simple symbols like `users` while keeping a shared namespace outside the generated file, configure a default namespace before importing those tables:

```ts
// db/kalam-orm.ts
import { configureKalamOrm } from '@kalamdb/orm';

configureKalamOrm({ namespace: 'app' });
```

```ts
// db/schema.generated.ts
import { getKalamTableConfig, kTable } from '@kalamdb/orm';
import { text } from 'drizzle-orm/pg-core';

export const users = kTable.shared('users', {
  id: text('id').primaryKey(),
});
export const usersConfig = getKalamTableConfig(users)!;
```

```ts
// db/index.ts
import './kalam-orm';

export * from './schema.generated';
```

Pass `{ systemColumns: true }` when your app needs typed `_seq`/`_deleted` fields for ordering, resume checks, or diagnostics. Streams only receive `_seq`; shared and user tables receive `_seq` and `_deleted`.

## Consumer And ORM Coverage

The package test suite includes dedicated generated-schema plus `runConsumer()` scenarios for chat, AI chat memory, job dispatch, support CRM, and commerce fulfillment domains. Each scenario combines shared, user, and stream table definitions with topic consumption and Drizzle builders through `kalamDriver()` or `executeAsUser()`.

## Generate `schema.ts`

```bash
kalamdb-orm \
  --url http://localhost:2900 \
  --user admin \
  --password AdminPass123! \
  --namespace app \
  --include-system-columns \
  --out src/db/schema.ts
```

Generator options:

- `--namespace <name>`: limit output to one or more namespaces. Repeat it or pass comma-separated names.
- `--include-system`: include `system` and `dba` tables.
- `--include-system-columns [all|_seq,_deleted]`: add KalamDB hidden system columns to generated table types.
- `--bigint-mode <string|bigint|number>`: choose how generated `BIGINT` columns are emitted. Default is `string` to preserve Int64 precision.
- `--no-type-aliases`: skip generated `$inferSelect` and `$inferInsert` aliases.

The generator introspects `SHOW TABLES`, uses `DESCRIBE` when column metadata is incomplete, preserves primary keys and non-null columns, and emits imports only for builders used by the generated schema. Generated schemas include `${tableName}Config`, `$inferSelect`, and `$inferInsert` exports next to each table, so browser apps and agents can import `schema.generated.ts` directly without a wrapper file.

When you generate schema, table exports are namespaced (`app_users` / `"app.users"`) even if you pass a single `--namespace`. That keeps generated files stable for agents and does not require `configureKalamOrm`. Pass `unqualifiedNames: true` (or keep hand-written short names) only when you want `users = kTable.user("users", ...)` plus `configureKalamOrm({ namespace: 'app' })` before importing the generated module.

## Watch `schema.ts` in local development

Add a generator script to your app:

```json
{
  "scripts": {
    "schema:gen": "kalamdb-orm --url http://localhost:2900 --user admin --password AdminPass123! --namespace app --out src/db/schema.ts"
  }
}
```

Then let the Kalam CLI rerun that script whenever `system.tables` changes:

```bash
# Watch one namespace
kalam --watch-schema --namespace app --run "npm run schema:gen" --run-on-start

# Watch several namespaces
kalam --watch-schema --namespace chat --namespace billing --run "npm run schema:gen"

# Watch one table only
kalam --watch-schema --table app.messages --run "npm run schema:gen"
```

`--watch-schema` polls every 5 seconds by default. Override that with `--interval 2s`, `--interval 500ms`, or another supported duration when you need faster feedback.

## KalamDB datatype mapping

| KalamDB type | Generated Drizzle helper | Wire/read note |
|---|---|---|
| `BOOLEAN` | `boolean()` | boolean |
| `INT` | `integer()` | number |
| `SMALLINT` | `smallint()` | number |
| `BIGINT` | `text()` by default | Int64 is transported as a string; use `--bigint-mode bigint` or `number` if desired |
| `DOUBLE` | `doublePrecision()` | number |
| `FLOAT` | `real()` | number |
| `TEXT` | `text()` | string |
| `TIMESTAMP` / `DATETIME` | `timestamp(..., { mode: 'date' })` | numeric timestamp is normalized to `Date` |
| `DATE` | `date(..., { mode: 'date' })` | date-day wire value is normalized to `Date` |
| `TIME` | `time()` | microseconds since midnight normalize to `HH:mm:ss[.fraction]` |
| `JSON` | `jsonb()` | plain JSON |
| `BYTES` | `bytes()` | `Uint8Array` |
| `EMBEDDING(n)` | `embedding(name, n)` | `number[]` |
| `UUID` | `uuid()` | string UUID |
| `DECIMAL(p,s)` | `numeric()` | exact decimal string |
| `FILE` | `file()` | `FileRef | null` |

## Live table helpers

```ts
import { liveTable } from '@kalamdb/orm';
import { messages } from './schema';

const stop = await liveTable(client, messages, (rows) => {
  console.log(rows);
}, { lastRows: 50 });

await stop();
```

`liveTable()` reuses the same `@kalamdb/client` connection and normalizes timestamp/date/time fields according to the Drizzle table definition.

`liveTable()` accepts the same row-oriented live options as `@kalamdb/client.live()`, including `lastRows`, `from`, `limit`, `getKey`, and `onCheckpoint` when you want to persist a resume cursor. Raw event streams stay in `@kalamdb/client.liveEvents()`.

## React Typed Mode

`@kalamdb/react` can compile the same Drizzle table descriptors into live query controllers:

```tsx
import { LiveQueries } from '@kalamdb/react';
import { asc, eq } from 'drizzle-orm';
import { messages, typing } from './schema.generated';

<LiveQueries
  queries={{
    messages: {
      table: messages,
      where: (table) => eq(table.conversationId, conversationId),
      orderBy: (table) => asc(table.createdAt),
      deps: [conversationId],
    },
    typing: {
      table: typing,
      where: (table) => eq(table.conversationId, conversationId),
      deps: [conversationId],
    },
  }}
>
  {({ messages, typing }) => <ChatView messages={messages.rows} typing={typing.rows} />}
</LiveQueries>
```

This keeps schema ownership in the ORM package and avoids a separate React-specific schema layer.

## Execute as a user

Agents and service workers can compile a Drizzle builder and run it through KalamDB's `EXECUTE AS USER` path:

```ts
import { executeAsUser } from '@kalamdb/orm';

await executeAsUser(
  client,
  db.insert(messages).values({
    room: 'main',
    role: 'assistant',
    author: 'KalamDB Copilot',
    sender_username: 'alice',
    content: 'Done.',
  }),
  'alice',
);
```

Only pass a user id that your service account is authorized to impersonate.

## FILE uploads from Drizzle

Pass upload bytes in `.values()` or `.set()` with `kalamFile()`. `kalamDriver()` detects them and routes to `queryWithFiles()` automatically — the same path as normal inserts:

```ts
import { kalamFile } from '@kalamdb/orm';

await db.insert(attachments).values({
  id: 'att_1',
  file_data: kalamFile('upload', selectedFile),
});
```

You can also pass a `File` or `Blob` directly to a generated `file()` column; the driver uses the blob name (or `upload`) as the multipart field name.

`queryWithFiles()` remains available for raw SQL strings. `compileQuery()` is exported when you need normalized SQL and params without executing.

## Kalam CLI workflow generation

When using the KalamDB project workflow, configure TypeScript output in `kalam.toml`:

```toml
[schema.targets.typescript]
output = "src/generated/kalam.ts"
```

Run `kalam schema gen` from the project root to regenerate `src/generated/kalam.ts` through `@kalamdb/orm` against the resolved workflow environment. The file is a generated artifact — do not edit it manually.

During `kalam dev`, TypeScript output regenerates automatically when `dev.generate_types = true` and the schema pipeline succeeds. Schema apply failures pause regeneration until you fix migrations and retry with `kalam dev --force`.

## License

Licensed under the Apache License, Version 2.0 (`Apache-2.0`). See the packaged `LICENSE.txt` and `NOTICE` files.
