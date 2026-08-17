# Example 2: mixed replication modes

One authenticated Kalam session can combine local-first tables and backend-authoritative replicas.

```dart
final kalam = await Kalam.open(
  url: serverUrl,
  auth: auth,
  tables: const [
    KalamTable(
      'app.todos',
      mode: KalamSyncMode.bidirectional,
    ),
    KalamTable(
      'app.messages',
      mode: KalamSyncMode.replicaOnly,
    ),
    KalamTable(
      'app.conversations',
      mode: KalamSyncMode.replicaOnly,
    ),
    KalamTable(
      'app.notifications',
      mode: KalamSyncMode.replicaOnly,
    ),
  ],
);
```

All four subscriptions are multiplexed over the one `kalam_link` connection. Each table has its own committed checkpoint and apply transaction.

## Bidirectional table

```dart
await kalam.table('app.todos').insert({
  'id': Kalam.id(),
  'title': 'Works while offline',
  'completed': false,
});
```

The insert is visible immediately and automatically queued for KalamDB.

## Replica-only tables

```dart
final messages = kalam.table('app.messages');

final Stream<List<KalamSyncedRow<KalamRow>>> rows =
    messages.watchWithSyncState(
  where: 'conversation_id = ?',
  args: [conversationId],
  orderBy: const ['created_at ASC'],
);
```

This is legal because it is a local read. A direct write is rejected:

```dart
await messages.delete(messageId);
// Throws KalamReplicaOnlyWriteException before changing SQLite.
```

To request a backend-owned deletion, enqueue a feature action:

```dart
await messageActions.delete(
  DeleteMessage(messageId: messageId),
);
```

The lifecycle is:

```text
enqueue action locally
  -> wait for connectivity
  -> call backend with stable idempotency key
  -> backend changes KalamDB
  -> KalamDB publishes the delete
  -> sync engine commits delete + checkpoint locally
  -> Drift watch refreshes the UI
```

There is no second Masky WebSocket, manual reconnect loop, or “delete locally again after HTTP succeeds” branch.

## Pending UI without changing the replica

If the app wants immediate feedback, it can observe the durable action:

```dart
final pendingDeletes = messageActions.delete.watchPending(
  key: (command) => command.messageId,
);
```

The widget can dim or label the matching message while it remains in the authoritative replica. This keeps the table truthful while still making offline intent visible.

For optimistic sends or deletes, an SDK-owned overlay is joined by `watchWithSyncState()`. It should not physically overwrite or delete the authoritative replica row. That distinction prevents a rejected backend action from corrupting the local mirror.

## Masky mapping

The likely initial policies are:

| Masky data | Suggested mode | Reason |
| --- | --- | --- |
| Messages | `replicaOnly` | REST/backend actions are authoritative; Kalam events update local rows |
| Conversations | `replicaOnly` | Backend/user-update events drive refresh and changes |
| Notifications | `replicaOnly` | Server-originated data |
| Event-change tables | `replicaOnly` or widget consumer | Depends on whether the UI queries history or only reacts to events |
| Pure client drafts/todos | `bidirectional` | Immediate offline CRUD is the desired behavior |

Masky can change one table at a time without changing the connection or duplicating transport logic.
