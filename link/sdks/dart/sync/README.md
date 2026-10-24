# kalam_sync

Local-first Flutter sync for [KalamDB](https://kalamdb.org). One Drift cache, live SQL, optimistic rows, and an offline action outbox on top of [`kalam_link`](https://pub.dev/packages/kalam_link).

→ **[Docs](https://kalamdb.org/docs/dart-sdk/sync)** · [Offline actions](https://kalamdb.org/docs/dart-sdk/sync-actions) · [kalam_link](https://pub.dev/packages/kalam_link) · [GitHub](https://github.com/kalamdb/KalamDB)

## Shutdown and replay load

Always await `Kalam.dispose()` before clearing application projection data. It
drains active consumer callbacks and action executions before closing SQLite.
The outbox runs at most eight independent ordering keys concurrently; queued
work remains durable when a session is disposed. Standalone action runners must
likewise await `KalamActionRunner.dispose()` before closing their store.

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

One account uses one Drift database. `Kalam.open` creates it, builds it with your database constructor, or adopts a database you already opened. To share the app database, mix `KalamSyncTablesMixin` into that `GeneratedDatabase`, include `kalamSyncSchema` in `allSchemaEntities`, and pass the opened database to `Kalam.open` and `Kalam.catchUp`. An adopted database stays open after `dispose`. `spec.liveSql` is the remote `SELECT`. `decodeChange` separates a full row, a partial patch, and a delete. `seedCheckpointIfAbsent` and `resetProjection` update the existing checkpoint and action tables. `planKalamSession` keeps the session when only the token changes.

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

`subject` is required. Cached rows and queued actions are keyed by server URL, namespace, and authenticated user.

## Sync a table

`kalam schema gen --languages dart` writes row codecs and `KalamTables.*`. Bind that spec, subscribe the SQL the screen needs, and write locally:

```dart
final todos = kalam.table(KalamTables.todos);

await kalam.subscribe(
  todos.consumer(sql: 'SELECT * FROM app.todos'),
);

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

Cancel the subscription when the widget unloads: `sync?.cancel()`.

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
