# kalam_sync

Local-first Flutter sync for [KalamDB](https://kalamdb.org). One Drift cache, live SQL, optimistic rows, and an offline action outbox on top of [`kalam_link`](https://pub.dev/packages/kalam_link).

→ **[Docs](https://kalamdb.org/docs/dart-sdk/sync)** · [Offline actions](https://kalamdb.org/docs/dart-sdk/sync-actions) · [kalam_link](https://pub.dev/packages/kalam_link) · [GitHub](https://github.com/kalamdb/KalamDB)

## Shutdown and replay load

Always await `Kalam.dispose()` before clearing application projection data. It
drains active consumer callbacks and action executions before closing SQLite.
The outbox runs at most eight independent ordering keys concurrently; queued
work remains durable when a session is disposed. Standalone action runners must
likewise await `KalamActionRunner.dispose()` before closing their store.

## Durable consumer actions

Inside `KalamEventConsumer.consume`, use `context.enqueueForEvent` for work
caused by a row change. Provide the stable consumer ID, row primary key, and
`change.seq`; the SDK derives an account-scoped action ID and deduplicates an
identical replay. Existing explicitly named actions remain untouched. A replay
with a genuinely different request still raises a conflict.

```dart
await context.enqueueForEvent(
  actionKey: 'refresh-conversation',
  consumerId: 'user-updates',
  rowKey: change.row['id'] as String,
  version: change.seq,
  payload: {'targetId': change.row['target_id']},
);
```

## Install

```bash
kalam init --yes --languages dart --template simple-live
kalam schema gen --languages dart
kalam dev
```

Or add the package to an existing app:

```bash
flutter pub add kalam_sync
```

## Get started

The starter commands above create a project with a schema and generated Dart
models. For an existing app, add a Kalam project and schema first, run schema
generation, and import its generated `lib/generated/kalam.dart` file. The
examples below use a generated `app.todos` table; replace its name and fields
with your own schema.

Open an account-scoped cache. The WebSocket stays lazy until a consumer or queued action needs it.

```dart
import 'package:flutter/material.dart';
import 'package:kalam_sync/kalam_sync.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();

  final kalam = await Kalam.open(
    url: 'http://localhost:2900',
    subject: 'dev-user',
    authProvider: () async => Auth.basic('root', 'kalamdb123'),
  );

  runApp(KalamScope(kalam: kalam, child: const App()));
}
```

`subject` must be the stable authenticated user ID, not the token or a shared
device ID. Cached rows and queued actions are keyed by server URL, namespace,
and subject. Use real user credentials in production; the root credentials here
are only an example for a local development server. On Android emulators,
`localhost` refers to the emulator, so use an address reachable by the device.

## Sync a table

`kalam schema gen --languages dart` writes row codecs and `KalamTables.*`. Bind that spec, subscribe the SQL the screen needs, and write locally:

```dart
final todos = kalam.table(KalamTables.todos);

final todoSync = await kalam.subscribe(todos.consumer());

await todos.insert(
  Todos(id: Kalam.id(), title: 'Ship it', completed: false),
  actionId: Kalam.id(),
);
```

`insert` / `update` / `delete` change the local row and enqueue DML in the same SQLite transaction. `watch()` keeps working offline.

```dart
StreamBuilder<List<KalamSyncedRow<Todos>>>(
  stream: todos.watchWithSyncState(),
  builder: (context, snapshot) {
    final rows = snapshot.data ?? const [];
    return ListView(
      children: [
        for (final row in rows)
          ListTile(
            title: Text(row.value.title),
            trailing: Icon(row.isSynced ? Icons.cloud_done : Icons.cloud_upload),
          ),
      ],
    );
  },
)
```

Keep the subscription handle for as long as the screen or service needs live
updates, then call `await todoSync.cancel()`. Call `await kalam.dispose()` when
the account session ends. `KalamScope` manages app pause/resume but does not
dispose the session. For a filtered remote subscription, pass `sql` and `params`
to `consumer`; a local `watch()` reads the cached projection.

## Custom actions

For backend-authoritative tables (`replicaOnly`), generate a durable queue with `kalam_sync_generator` instead of calling `insert` directly:

```dart
await actions.sendMessage(
  SendMessageArgs(
    messageId: message.id,
    conversationId: conversationId,
    text: message.text,
    createdAt: message.createdAt,
    author: message.author,
  ),
  orderingKey: conversationId,
  optimistic: messages.optimisticInsert(message),
);
```

Walkthrough: [Offline actions](https://kalamdb.org/docs/dart-sdk/sync-actions).

## Links

- Docs: [https://kalamdb.org/docs/dart-sdk/sync](https://kalamdb.org/docs/dart-sdk/sync)
- Chat example: [`example/`](example)
- License: Apache-2.0

## Durable retries and checkpoint initialization

An action ID identifies one request across the shared store. Re-enqueuing it with
the same account, action key, version, kind, payload JSON, ordering key, and target
row returns the existing action without repeating optimistic work. Reusing it for
a different request throws `StateError`. Keep the serialized payload stable on
retry; the store compares the persisted JSON text. This is local deduplication,
not a promise of exactly-once remote execution.

`seedCheckpointIfAbsent` checks and inserts inside one transaction. Concurrent
seeds produce one winner; an existing checkpoint is never advanced by seeding.
`subscriptionId` names the checkpoint, while `tableId` supplies the quoted table
in `KalamTableSpec.liveSql`. Delete decoding rejects an event missing its key.
Generated specs also support `spec.bind(client)` for remote typed queries using
the same decoder and key metadata.

## Generated typed remote queries

After `kalam schema gen --languages dart`, reuse the generated table codecs and
primary key directly:

```dart
final inventory = KalamTables.inventory.bind(client);
final query = inventory.query()
    .filter(InventoryColumns.sku.eq('sku-1'))
    .limit(20);
final rows = await query.get();
final updates = query.live();
```

`InventoryColumns.sku` is `KalamColumn<String>`, so passing a number fails Dart
analysis. Existing `.where((c) => c.col('sku').eq(value))` remains available for
dynamic queries. `eq(null)` and `neq(null)` produce `IS NULL` and `IS NOT NULL`
when the column type permits null. Values use bound parameters.

`orderBy` on a remote live query sorts the matching rows locally before applying
`limit`. The limit controls the displayed rows, not retained memory. Use a narrow
filter for large datasets. Scalar string ordering is local and does not promise
server collation equivalence. Use `kalam_sync` for durable local projections and
offline actions; `bind(client)` uses remote transport and does not add persistence.

Generated Dart table specs require one declared primary key. Keyless tables still
get row codecs, but no sync spec or guessed key. Composite primary keys are rejected
by schema compilation to match current server support. Array codecs preserve null
arrays and encode/decode each supported scalar, enum, composite, and timestamp
element; null array elements are not represented by the current contract.
## Background catch-up

Use `Kalam.catchUp` for a bounded background pass, such as an FCM wake. Pass
the same account identity, database and consumers used by the foreground:

```dart
final result = await Kalam.catchUp(
  url: serverUrl,
  subject: userId,
  namespace: 'public',
  authProvider: loadAuth,
  database: database,
  consumers: consumers,
  mode: KalamCatchUpMode.snapshot,
  rowLimit: 100,
  timeout: const Duration(seconds: 30),
  flushOutbox: false,
);
```

Snapshot mode reads the current table snapshot and applies only rows newer
than the durable checkpoint. It stops after the server's final initial batch,
including an empty batch. It can scan older rows and does not reconstruct
hard-deleted rows or intermediate changes, so use it only for projections that
can recover from current state. Replay mode preserves event history but the
server can deliver replay after the initial snapshot marker; it is bounded by
the deadline or row limit, and may return `timedOut`. Consumers run in order.
The owned transport is closed after the pass;
an explicitly supplied database or transport remains owned by its caller.
Check `timedOut`, `hasMore`, and `appliedByConsumer`: a timeout or row limit
means the pass may be incomplete and another catch-up is needed. A row limit
is applied per consumer, not across the entire pass. `flushOutbox: false`
leaves queued actions for a later foreground session.
At the deadline, an in-flight consumer finishes its database transaction before
the pass returns, so cleanup can take longer than the configured timeout.

Avoid wrapping the pass in another timeout with the same deadline: Dart's
`Future.timeout` does not cancel the underlying sync and can race cleanup.
