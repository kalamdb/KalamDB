# Offline messaging with one conversation

This is the intended minimal messaging flow when an application backend, rather than direct KalamDB DML, owns message creation and deletion.

## What the application supplies

The application supplies only:

- the conversation filter;
- the backend calls for `sendMessage` and `deleteMessage`;
- any domain fields required by those calls;
- the widget presentation.

`kalam_sync` supplies the local cache, shared connection, durable subscription checkpoint, optimistic row, outbox, retry, reconciliation, and row sync state.

## Configure the backend-authoritative table

```dart
final messageSync = AppTables.messages.replicaOnly(
  where: (message) =>
      message.conversationId.equals(conversationId),
);

final kalam = await Kalam.open(
  url: serverUrl,
  auth: auth,
  tables: [messageSync],
);

final messages = messageSync.bind(kalam);
```

The sync engine subscribes to the filtered backend changes and applies them to SQLite. The widget does not consume raw WebSocket events; it watches the local Drift-backed view:

```dart
final Stream<List<KalamSyncedRow<Message>>> rows =
    messages.watchWithSyncState(
  orderBy: (message) => message.createdAt.asc(),
);
```

`replicaOnly` prevents arbitrary direct writes, but an installed custom action may create an SDK-managed optimistic row. That exception is explicit and atomic with the action enqueue.

## Define the two backend actions

Table schema generation does not know which HTTP endpoints to call. Therefore it does not generate executable `CreateMessageAction` and `DeleteMessageAction` classes from the table alone.

The action generator removes registry and serialization boilerplate, while the application provides the unavoidable backend behavior:

```dart
@KalamActionPayload()
class SendMessageArgs {
  const SendMessageArgs({
    required this.messageId,
    required this.conversationId,
    required this.text,
    required this.createdAt,
  });

  final String messageId;
  final String conversationId;
  final String text;
  final DateTime createdAt;
}

@KalamActionPayload()
class DeleteMessageArgs {
  const DeleteMessageArgs({required this.messageId});

  final String messageId;
}

@KalamActionModule(namespace: 'messages')
class MessageActions with _$MessageActions {
  MessageActions(this.api);

  final MessageApi api;

  @KalamAction(name: 'send', version: 1)
  late final send = queuedAction<SendMessageArgs>(
    optimistic: (local, args) => local
        .replica(AppTables.messages)
        .insert(
          MessagesCompanion.insert(
            id: args.messageId,
            conversationId: args.conversationId,
            text: args.text,
            createdAt: args.createdAt,
          ),
        ),
    run: (context, args) => api.sendMessage(
      messageId: args.messageId,
      conversationId: args.conversationId,
      text: args.text,
      idempotencyKey: context.idempotencyKey,
    ),
  );

  @KalamAction(name: 'delete', version: 1)
  late final delete = queuedAction<DeleteMessageArgs>(
    optimistic: (local, args) => local
        .replica(AppTables.messages)
        .markPendingDelete(args.messageId),
    run: (context, args) => api.deleteMessage(
      messageId: args.messageId,
      idempotencyKey: context.idempotencyKey,
    ),
  );
}
```

The local optimistic write is not a second DML action. `local.replica(...)` is available only inside the action's enqueue transaction and records the row state against that custom action.

If the backend writes directly through KalamDB and no custom API is needed, configure the table as `bidirectional` and use its Drift-backed `insert/update/delete` methods instead. No `MessageActions` module is necessary in that simpler case.

## Send while offline

```dart
Future<void> sendMessage(String text) {
  final now = DateTime.now().toUtc();

  return messageActions.send(
    SendMessageArgs(
      messageId: Kalam.id(),
      conversationId: conversationId,
      text: text,
      createdAt: now,
    ),
  );
}
```

The call returns after one SQLite transaction commits:

1. the optimistic message values;
2. the per-row `pending` state;
3. the serialized `messages.send@1` outbox action.

No connection check occurs in the repository or widget.

## Display per-message state

```dart
Widget buildMessage(KalamSyncedRow<Message> row) {
  return MessageBubble(
    text: row.value.text,
    status: switch (row.sync.phase) {
      KalamRowSyncPhase.pending => 'Waiting to send',
      KalamRowSyncPhase.sending => 'Sending…',
      KalamRowSyncPhase.awaitingServerEcho => 'Sent, syncing…',
      KalamRowSyncPhase.synced => null,
      KalamRowSyncPhase.failed => 'Not sent — tap to retry',
    },
    onRetry: row.sync.phase == KalamRowSyncPhase.failed
        ? () => kalam.actions.retry(row.sync.actionId)
        : null,
  );
}
```

The sync phase is SDK-owned local metadata. It must not be confused with backend domain delivery states such as `sent`, `delivered`, and `read`.

## Reconcile the backend echo

When connectivity returns:

1. the outbox executes `api.sendMessage` using the stable action UUID as its idempotency key;
2. the backend performs its workflow and writes the authoritative message into KalamDB;
3. KalamDB publishes the table change;
4. `kalam_sync` applies the row and checkpoint atomically;
5. the optimistic row is reconciled and its sidecar state becomes `synced`.

The cleanest contract keeps the client-generated `messageId` as the authoritative primary key. If the backend must allocate another ID, it must persist and echo a unique `clientMessageId` or action UUID so the SDK can reconcile instead of showing a duplicate message.

An acknowledged HTTP request is only `awaitingServerEcho`. It becomes `synced` after the matching KalamDB change is committed locally.

## Why sync state is a sidecar

Do not append `_kalam_synced`, `_kalam_error`, and `_kalam_action_id` columns to every generated table. A Kalam-owned sidecar avoids remote-schema collisions, repeated migrations, and accidental upload of local-only values.

Conceptually it contains:

```text
account + table_id + row_key
action_id
phase
attempt_count + next_retry_at
last_error
last_server_seq
pending values or delete tombstone when required
```

`watchWithSyncState()` performs the merge and exposes a typed `KalamSyncedRow<Message>`. Apps that do not need status can use `messages.watch()` and receive only Drift-generated `Message` values.

## Multiple backend endpoints

One action may coordinate several endpoints, but the executor is retried at least once. Every externally visible step therefore needs an idempotency key. The SDK can offer durable named steps:

```dart
run: (context, args) async {
  final upload = await context.step(
    'upload-attachment',
    run: (stepKey) => api.uploadAttachment(
      args.attachment,
      idempotencyKey: stepKey,
    ),
  );

  await context.step(
    'send-message',
    run: (stepKey) => api.sendMessage(
      messageId: args.messageId,
      attachmentId: upload.id,
      idempotencyKey: stepKey,
    ),
  );
}
```

The step key is derived from the action UUID and stable step name. A completed step is not intentionally repeated after restart, but an endpoint must still be idempotent because its response can be lost after the server commits. When possible, one backend endpoint that transactionally owns the whole workflow is safer than client orchestration.

