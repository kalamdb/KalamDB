# kalam_sync

Local-first Flutter sync for [KalamDB](https://kalamdb.org). One Drift cache, live SQL, optimistic rows, and an offline action outbox on top of [`kalam_link`](https://pub.dev/packages/kalam_link).

→ **[Docs](https://kalamdb.org/docs/dart-sdk/sync)** · [Offline actions](https://kalamdb.org/docs/dart-sdk/sync-actions) · [kalam_link](https://pub.dev/packages/kalam_link) · [GitHub](https://github.com/kalamdb/KalamDB)

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
