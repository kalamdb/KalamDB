# Example 1: bidirectional todos

This is the smallest local-first Flutter shape. There is no action module because ordinary KalamDB row writes already have a built-in durable action.

## Open the session

```dart
import 'package:flutter/material.dart';
import 'package:kalam_sync/kalam_sync.dart';

Future<Kalam> openKalam(Auth auth) {
  return Kalam.open(
    url: 'https://db.example.com',
    auth: auth,
    tables: const [
      KalamTable(
        'app.todos',
        mode: KalamSyncMode.bidirectional,
      ),
    ],
  );
}
```

The SDK derives the authenticated subject after login and opens the matching per-user cache. It starts local reads immediately, resumes inbound changes from the committed table checkpoint, and flushes pending actions when the connection is ready.

## Keep feature methods small

```dart
class TodoRepository {
  TodoRepository(Kalam kalam) : todos = kalam.table('app.todos');

  final KalamTableRef todos;

  Stream<List<KalamRow>> watchTodos() {
    return todos.watch(orderBy: const ['created_at DESC']);
  }

  Future<void> add(String title) {
    return todos.insert({
      'id': Kalam.id(),
      'title': title,
      'completed': false,
      'created_at': DateTime.now().toUtc(),
    });
  }

  Future<void> setCompleted(String id, bool completed) {
    return todos.update(
      id,
      {'completed': completed},
    );
  }

  Future<void> delete(String id) {
    return todos.delete(id);
  }
}
```

Each write returns after SQLite commits. It does not wait for the network. Internally, the row change and built-in DML outbox item are one transaction. No repository method calls `isConnected`, retries a request, or manages the socket.

## Render from SQLite

```dart
class TodoList extends StatelessWidget {
  const TodoList({
    super.key,
    required this.todos,
    required this.kalam,
  });

  final TodoRepository todos;
  final Kalam kalam;

  @override
  Widget build(BuildContext context) {
    return Column(
      children: [
        ValueListenableBuilder<KalamSyncState>(
          valueListenable: kalam.syncState,
          builder: (context, state, _) => Text(
            switch (state.phase) {
              KalamSyncPhase.offline =>
                'Offline — ${state.pendingActions} pending',
              KalamSyncPhase.catchingUp => 'Updating…',
              KalamSyncPhase.flushing =>
                'Sending ${state.pendingActions} changes…',
              KalamSyncPhase.live => 'Up to date',
              KalamSyncPhase.error => 'Sync needs attention',
            },
          ),
        ),
        Expanded(
          child: StreamBuilder<List<KalamRow>>(
            stream: todos.watchTodos(),
            builder: (context, snapshot) {
              final rows = snapshot.data ?? const [];
              return ListView.builder(
                itemCount: rows.length,
                itemBuilder: (context, index) {
                  final row = rows[index];
                  return CheckboxListTile(
                    value: row.boolValue('completed'),
                    title: Text(row.stringValue('title')),
                    onChanged: (value) => todos.setCompleted(
                      row.stringValue('id'),
                      value ?? false,
                    ),
                  );
                },
              );
            },
          ),
        ),
      ],
    );
  }
}
```

The widget does not subscribe to WebSocket events. It watches the local table. The sync engine applies remote changes to that table, and Drift invalidation refreshes the widget.

## Expected conflict contract

The v1 default should be explicit and deterministic:

- the client assigns the mutation/action ID;
- the server applies an idempotent mutation;
- the authoritative server event reconciles the optimistic local row;
- a delete uses a tombstone until the server result is known, so a failed action can be surfaced or restored;
- a permanent rejection marks the action failed and exposes it through `syncState`/`actions.watchFailed()`.

A later conflict hook may support merge policies, but the default cannot silently be “last callback happened to win.”

