# Paired developer experience and implementation handoff

Slice 1 examples are the TypeScript Drizzle calls and the Dart table, database-mode, and session helpers. They compile against today's generator. Slice 2 examples (generator API v2, operation descriptors, replay cursors, automatic durable retries) stay in this file and are not part of the first ship. T044 turns Slice 2 examples into fixtures only after the Slice 1 gate. Never hand-edit generated output to make an example pass.

## 1. Generate once from the project contract

Keep the existing workflow:

```sh
kalam init --yes --languages typescript,dart
kalam schema gen --languages typescript,dart
kalam dev --agent
```

Run in a disposable project, using its isolated server ports and credentials. Initialization may produce a language scaffold; the acceptance harness must also create a real Flutter application and attach the generated package using the supported project layout. New templates select generator API version 2. Existing projects explicitly select `api_version = 2` in each language target configuration before migration; T042 must document the exact configuration placement. No language-wide SDK breaking change is hidden in regeneration.

Add `kalam schema gen --check` for CI. It exits nonzero when owned output differs and writes nothing. Generation validates all selected targets before publishing output, preserves unrelated files, and reports recoverable interrupted publication. Do not claim a filesystem offers a cross-directory atomic rename: use staged versioned output plus a commit marker/recovery protocol where required.

The fixture defines:

| Definition | Contract |
| --- | --- |
| `chat.messages` | `id TEXT PRIMARY KEY`, `conversation_id TEXT NOT NULL`, `body TEXT NOT NULL`, `created_at TIMESTAMPTZ NOT NULL DEFAULT now()`, `edited_at TIMESTAMPTZ NULL` |
| `chat.send_message` | Required `conversation_id`, `text`; returns a declared result containing the created message; uses generated server handler/input/result contracts |
| `chat.messages_by_conversation` | Named parameterized query returning message rows; keyed single-table live eligibility; distinct from a routine |
| Durable routine policy | `send_message` is marked durable only after T034 proves effects and receipt commit together; ordinary routines remain immediate-only |

Use actual supported project schema/routine declarations when implementing the fixture. The names above are fixture requirements, not invented SQL annotation syntax. Include a second table with a composite key and a non-key column named `id` in conformance tests.

## 2. Corresponding typed queries

TypeScript stays on Drizzle. `kalam dev` generates `schema.ts` as it does today:

```ts
import { createClient } from '@kalamdb/client';
import { kalamDriver } from '@kalamdb/orm';
import { desc, eq } from 'drizzle-orm';
import { drizzle } from 'drizzle-orm/pg-proxy';
import { messages } from './generated/schema';

const client = createClient({ url: serverUrl, authProvider: resolveAuth });
const db = drizzle(kalamDriver(client));
const rows = await db
  .select()
  .from(messages)
  .where(eq(messages.conversationId, conversationId))
  .orderBy(desc(messages.createdAt))
  .limit(50);
```

Dart:

```dart
import 'package:kalam_link/kalam_link.dart';
import 'generated/kalam.dart';

final client = createClient(url: serverUrl, authProvider: resolveAuth);
final db = createKalam(client);
final query = db.tables.chat.messages.query()
    .where((m) => m.conversationId.eq(conversationId))
    .orderBy((m) => m.createdAt.desc())
    .limit(50);
final rows = await query.get();
```

`serverUrl`, `resolveAuth`, and `conversationId` belong to the application. Authentication providers use each SDK's existing credential abstraction. Both queries return generated row models, decode all fields, and preserve exact SQL names through metadata. Both clients are lazy and owned by the surrounding session.

Named queries avoid reconstructing more complex SQL in application code:

```ts
const rows = await createKalam(client).queries.chat.messagesByConversation({ conversationId }).get();
```

```dart
final rows = await db.queries.chat.messagesByConversation(
  ChatMessagesByConversationInput(conversationId: conversationId)).get();
```

## 3. The same query becomes live

TypeScript:

```ts
import { liveTable } from '@kalamdb/orm';

const stop = await liveTable(client, messages, (rows) => render(rows), { limit: 50 });
await stop();
```

Dart:

```dart
final live = query.live();
final subscription = live.stream.listen(
  (state) => render(state.rows, state.status));
// On owner disposal:
await subscription.cancel();
await live.dispose();
```

Disposal belongs in lifecycle cleanup, not immediately after observation in a real application. First observation starts delivery and receives seeded state. `connected`, `isComplete`, and `isFromCache` answer different questions. Handle ownership does not grant ownership of another observer's underlying shared subscription.

For React, the existing hook gains `useLiveQuery({ query })`; memoize or otherwise stabilize the query definition. For Flutter, create/own the handle outside `build` and use its replaying stream plus `snapshot` as initial data. Do not open a socket or database for each widget render.

Here `limit(50)` means an exact presentation window in live mode. The runtime retains full eligible membership or uses an explicitly supported server window protocol. A default capacity error is preferable to silently losing the 51st row needed after a deletion. Large/infinite feeds must select a supported bounded strategy explicitly.

## 4. CRUD and function calls remain typed

TypeScript:

```ts
import { createKalam } from './generated/kalam.js';

await db.insert(messages).values({ id, conversationId, body: 'Hello' });
await db.update(messages).set({ editedAt: null }).where(eq(messages.id, id));
const message = await db.select().from(messages).where(eq(messages.id, id));
const result = await createKalam(client).functions.chat.sendMessage({ conversationId, text: 'Hello' });
```

Dart:

```dart
await db.tables.chat.messages.insert(
  ChatMessagesInsert(id: id, conversationId: conversationId, body: 'Hello'));
await db.tables.chat.messages.update(
  id, ChatMessagesPatch(editedAt: Field.value(null)));
final message = await db.tables.chat.messages.get(id);
final result = await db.functions.chat.sendMessage(
  ChatSendMessageInput(conversationId: conversationId, text: 'Hello'));
```

These are separate usage demonstrations, not a recipe to send the same message twice. `createdAt` is omitted so its server default applies; omitted patch fields stay unchanged. A single-column key accepts its generated scalar/domain type; composite keys require a generated key object. Generated field names follow the shared naming contract. Both function methods delegate to operation descriptors and return validated declared results.

Function-only applications import `createKalamFunctions` from the generated `functions` entry point, with no ORM or sync import. `kalam dev` and `kalam schema gen` keep emitting the Drizzle `kTable` schema for TypeScript tables. Dart table calls use `createKalam` from the generated facade.

## 4A. Native typed RPC, transport selection and server-side chaining (proposed Slice 2)

The generated function syntax shown above is unchanged, but API v2 implementations **must not emit** SQL `SELECT` / `CALL` text for the generated client invocation. Instead, a generated operation descriptor invokes the current function service through a native typed executor, using a negotiated JSON or binary encoding. Both modes preserve SQL types and error semantics. The SQL API is still separately available.

```ts
// Proposed v2: the generated method chooses typed JSON/binary internally.
const db = createKalam(client);
const reply = await db.functions.chatDemo.sendMessage({
  target: "room", targetId: "main", content: "Hello"
});
// Optional explicit transport preference on the lower-level base SDK:
const reply2 = await client.invoke(appOperations.chatDemo.sendMessage, {
  target: "room", targetId: "main", content: "Hello"
}, { format: "binary" });
```

Nested procedure composition is server-side, not a client-defined arbitrary function graph:

```ts
// Illustrative project-defined routines, both generated from schema.
export const bookAndBill = procedure.school.bookAndBill(async (ctx, input) => {
  const booking = await ctx.functions.school.bookLesson(input);
  return ctx.functions.school.createInvoice({ bookingId: booking.id });
});
```

The two nested calls remain in-process under the root transaction and permissions. Cross-Raft-data-group transactional writes fail explicitly, and STREAM/external side effects are not part of rollback guarantees. The native typed-value/read-view optimization and binary codec are **not** implemented until T051–T063 and Gate B2 pass, including prior codec security prerequisites. Review [API](contracts/public-api.md), [protocol](contracts/protocol.md), [tasks](tasks.md), and [037 ABI](../037-pg18-compatibility/contracts/functions-v8.md).

## 4B. Migrate WebSocket live traffic to the same binary codec (proposed Slice 2)

The current live WebSocket implementation negotiates JSON text or MessagePack binary; that **does not yet mean** it supports function RPC's future Kalam Binary codec. In the target API, both typed RPC and live subscriptions use the same Rust `kalamdb-serialization` value encoder/decoder through `link-common`, native SDK, browser WASM and Dart FRB, with distinct RPC and live message kinds. No per-event generic JSON tree is needed for the binary path.

Migration order: (1) harden decoder/decompression/V8 lifetime and fuzz, (2) build lightweight shared Rust codec, (3) finish typed JSON/binary function RPC with benchmarks, (4) negotiate Kalam Binary WebSockets and migrate SDKs, then **remove MessagePack completely from WebSocket/SDK live code** after a documented old-client compatibility window, (5) assess persisted/Raft migration separately. Old MessagePack bytes must never be treated as Kalam Binary; unsupported clients receive an upgrade error after retirement. No implicit zero-copy promise exists across network/WASM/FRB; eligible native borrowed views require validation.

This is specification-only until tasks T064–T081, prior T051–T063 and the new security/live sunset gates pass. See [binary/wire protocol](contracts/protocol.md), [execution plan](plan.md), and [research findings](research.md).

## 5. Add Dart persistence to the same query

```dart
final sync = await Kalam.open(config: appSyncConfig);
final db = createKalam(sync.remote);
final fullTable = sync.mirror(db.tables.chat.messages.query());
final filtered = sync.mirror(
  db.tables.chat.messages.query().where((m) => m.conversationId.eq(conversationId)));
final subscription = filtered.stream.listen(renderSnapshot);

// Kalam still opens the one connection. The app schema adds local tables.
final syncWithDrafts = await Kalam.open(
  config: appSyncConfig, database: AppDatabase.new);

// Existing Drift database, already including Kalam sync tables.
final syncAttached = await Kalam.open(
  config: appSyncConfig, database: existingAppDatabase);
```

`appSyncConfig` declares account/auth resolution, generated contract, Drift database mode, lifecycle policy, and selected mirror/consumer definitions. Generated sync definitions remove handwritten model/operation registries. `sync.remote` shares the session's client and is owned by the sync session; applications do not separately dispose it. `sync.database` is the one Drift database. Mirror cache reads work while offline. A mirror uses the durable materializer as its source of truth instead of keeping a second full in-memory live cache or a second SQLite file. `fullTable` resumes the whole table from the server cursor.

Queue a generated, explicitly durable operation:

```dart
final action = await sync.enqueue(
  db.operations.chat.sendMessage(
    ChatSendMessageInput(conversationId: conversationId, text: 'Hello')),
  orderingKey: conversationId);
final result = await action.result;
```

The enqueue call persists input before returning; the result may arrive after reconnect. Save `action.id` if a UI needs to restore this particular operation after restart. `sync.actions.restore(id, operation: appOperations.chat.sendMessage)` validates the saved operation version before decoding its result. Generated `appOperations` contains unbound operation descriptors; `db.operations` binds supplied input to the session. Completion and mirror reconciliation remain separately observable.

Background execution uses `Kalam.catchUp(config: appSyncConfig, budget: budget)`. It claims the same fenced work, shares definition/codec configuration, and reports actual received/applied/completed progress. It does not register a second independent copy of the foreground pipeline.

## 6. Prove the entire developer path

For each selected supported platform, the implementing agent must run:

1. Initialize a new project, generate both targets, compile application code and generated server handlers, and verify application health after server/schema readiness.
2. Insert/read/update/delete through both generated clients, compare results, and call the same routine through SQL/default SDK transport and its documented HTTP route.
3. Observe both live clients together; exercise empty snapshots, partial changes, filter exits, reconnect, disposal, and two observers of one query.
4. Restart the Flutter app offline; show cached rows, preserve pending input/results, reconnect, and resolve/reconcile without duplicate effects.
5. Add a nullable field and regenerate; change a routine input incompatibly and verify the compiler error and stale-artifact check; verify failed generation preserves the previous output.
6. Exercise unsupported query/server capabilities and expired cursors/idempotency windows. Inspect actionable errors instead of accepting empty-success fallbacks.
7. Run the hardening, shared-codec, RPC, WebSocket MessagePack-removal, crash/conformance/performance gates in [plan.md](plan.md), then migrate Masky with the recorded before/after glue baseline. Persisted/Raft byte migration has separate release/rollback gates.

## 7. Prompt for the implementation agent

Paste this into the agent that will implement the work:

```text
Use $speckit-implement for specs/038-unified-sdk-contracts.
Read spec.md Delivery order, plan.md, and the Slice 1 list in tasks.md.
Implement only T001, T019, T018, T038, T045, T046, and T048. Stop at the
Slice 1 gate. Preserve existing unrelated edits. Keep TypeScript on Drizzle
and keep current Dart client methods. Add the Dart table builder, one Drift
connection, and the Masky session/checkpoint/projection helpers on today's
wire format and sync tables. Do not start generator API v2, live replay
cursors, the sync storage rewrite, or server idempotency.
```

**For a Slice 2 implementation agent after Slice 1**, follow T051 and the ordered prerequisites T064–T071 before native RPC T052–T063, then WebSocket T072–T078 and independently gated storage/Raft work T079–T081. Do not treat existing WebSocket MessagePack as Kalam Binary; remove it entirely only after the documented wire compatibility gate and test coverage.

The active `.specify/feature.json` points to this feature. To validate the handoff location without implementing it:

```sh
bash .specify/scripts/bash/check-prerequisites.sh --json --require-tasks --include-tasks
```
