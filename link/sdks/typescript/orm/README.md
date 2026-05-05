# @kalamdb/orm

Drizzle ORM driver, KalamDB table helpers, live table helpers, and schema generator for KalamDB.

Use this package with `@kalamdb/client` when you want Drizzle-style queries in a browser app, admin UI, or Node service. Use `@kalamdb/consumer` separately for topic workers and agents.

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
  url: 'http://localhost:8080',
  authProvider: async () => Auth.basic('admin', 'AdminPass123!'),
});

const db = drizzle(kalamDriver(client));
const rows = await db.select().from(messages).limit(20);
```

The driver normalizes KalamDB temporal wire values for Drizzle columns: `timestamp(..., { mode: 'date' })`, `date(..., { mode: 'date' })`, and `time(...)` are converted from the numeric wire representation when needed.

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

Pass `{ systemColumns: true }` when your app needs typed `_seq`/`_deleted` fields for ordering, resume checks, or diagnostics. Streams only receive `_seq`; shared and user tables receive `_seq` and `_deleted`.

## Generate `schema.ts`

```bash
kalamdb-orm \
  --url http://localhost:8080 \
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
}, { subscriptionOptions: { last_rows: 50 } });

await stop();
```

`liveTable()` and `subscribeTable()` reuse the same `@kalamdb/client` connection and normalize timestamp/date/time fields according to the Drizzle table definition.

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
