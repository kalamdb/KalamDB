# Kalam Sync Flutter API proposal

Status: proposed public API; none of the types in this folder are implemented yet.

This proposal makes `kalam_sync` the local-first layer above `kalam_link`. A Flutter app opens one authenticated Kalam session, chooses a replication policy per table, and lets the SDK own the socket, durable checkpoints, local cache, retry queue, and connection state.

The design intentionally separates three concerns:

1. **Table replication policy** is declared once when the authenticated session opens.
2. **Outbound business actions** live in small, feature-owned action modules and are installed for the session.
3. **Event consumers** are created where they are needed, including inside a widget, and are cancelled when that feature unloads.

There is no central `events: []` list in `Kalam.open()`.

## Recommended public shape

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
  ],
);
```

The same connection and local SQLite database serve every table. The mode belongs to a table, not to the connection.

| Mode | Local read | Direct local write | Outbound behavior | Inbound behavior |
| --- | --- | --- | --- | --- |
| `bidirectional` | SQLite | Allowed | The row change and a built-in Kalam DML action are committed atomically | Server events reconcile the cached row |
| `replicaOnly` | SQLite | Rejected | Use a queued feature action when the backend must do work | Only backend events change the cached row |

`replicaOnly` is the Masky-style backend-authoritative mode. For example, deleting a message queues `messages.delete`; the local message remains until the backend accepts the action, changes KalamDB, and KalamDB sends the resulting delete event back to the app.

### Per-row local sync state

Pending/failed state should not add `_synced` columns to every server-shaped Drift table. The SDK owns a sidecar keyed by account, remote table ID, and row primary key. It tracks the action ID, local phase, attempts/error, and last applied server sequence. For an optimistic row that does not exist on the backend yet, the SDK also retains its pending local values or tombstone until reconciliation.

The typed API joins that metadata for the application:

```dart
Stream<List<KalamSyncedRow<Message>>> rows =
    messages.watchWithSyncState();
```

`KalamSyncedRow.value` is the Drift-generated `Message`; `KalamSyncedRow.sync.phase` can be `pending`, `sending`, `awaitingServerEcho`, `synced`, or `failed`. These phases describe local synchronization only. Domain states such as delivered/read remain ordinary backend-owned message columns.

## Why actions are feature modules

An outbox executor cannot safely belong to a widget. A queued action must still be executable after navigation, reconnection, or app resume. It therefore lives for the authenticated session, but its implementation stays beside its feature:

```text
lib/features/messages/
  message_actions.dart       action definitions and backend execution
  message_repository.dart    feature queries and ordinary methods
  message_screen.dart        widget-scoped local watches/event consumers
```

The app installs one module per feature:

```dart
final messageActions = await kalam.actions.install(
  MessageActions(messageApi),
);
```

Installation registers serialization and execution. The returned module exposes typed, callable actions:

```dart
await messageActions.delete(
  DeleteMessage(messageId: message.id),
);
```

That call only commits an outbox item. The SDK decides when to execute it, retains the same idempotency key across retries, and waits for the backend event to update a `replicaOnly` table.

## Annotation design

Annotations should remove codec and registry boilerplate, not hide whether a method executes now or is queued. Annotating an ordinary `Future<void> delete()` method is misleading because Dart cannot transparently replace that method call without generating a second facade.

The recommended generated form therefore annotates a **callable action field**:

```dart
part 'message_actions.kalam.dart';

@KalamActionModule(namespace: 'messages')
class MessageActions with _$MessageActions {
  MessageActions(this.api);

  final MessageApi api;

  @KalamAction(name: 'delete', version: 1)
  late final delete = queuedAction<DeleteMessage>(
    run: (context, command) => api.deleteMessage(
      command.messageId,
      idempotencyKey: context.idempotencyKey,
    ),
  );
}
```

The generator creates the module registry, stable action key (`messages.delete@1`), payload codec, and install adapter. `delete(command)` always means enqueue; only the SDK invokes `run` while flushing the outbox.

The annotations should be exported by `kalam_sync`, so application code still has one runtime dependency. Only generated action modules add development dependencies:

```yaml
dependencies:
  kalam_sync: ^0.6.0

dev_dependencies:
  build_runner: ^2.0.0
  kalam_sync_generator: ^0.6.0
```

A one-file application using built-in bidirectional table writes needs no `build_runner`. A manual `KalamQueuedAction.json(...)` escape hatch should remain available for apps that do not want generation.

## Custom optimistic actions

A custom backend workflow may need to change the UI immediately and enqueue one non-SQL action. The action definition can provide an optional local phase:

```dart
@KalamAction(name: 'deleteViaWorkflow', version: 1)
late final deleteViaWorkflow = queuedAction<DeleteTodo>(
  optimistic: (local, command) =>
      local.table('app.todos').delete(command.todoId),
  run: (context, command) => api.deleteTodoWorkflow(
    command.todoId,
    idempotencyKey: context.idempotencyKey,
  ),
);
```

The SDK commits the optimistic change and custom action in one SQLite transaction. Writes made through this `local` context do not create a second built-in DML outbox item. Arbitrary writes through another Drift connection are not part of this guarantee.

An action cannot directly mutate the authoritative base rows of a `replicaOnly` table. It may omit `optimistic`, or it may write through the SDK's explicit replica-overlay context. That context stores pending values/tombstones and row state beside the replica, and `watchWithSyncState()` merges them for the UI without turning them into a second built-in DML action.

## Widget-scoped events

Use the local table watch for ordinary UI rendering. Use an event consumer only when receiving a server change must trigger work:

```dart
late final KalamEventSubscription eventSubscription;

@override
void initState() {
  super.initState();

  eventSubscription = kalam.events.consume(
    id: 'conversation/${widget.conversationId}/delivery',
    query: '''
      SELECT *
      FROM app.message_events
      WHERE conversation_id = $1
    ''',
    params: [widget.conversationId],
    onEvent: (change, context) async {
      await context.enqueue(
        messageActions.acknowledgeDelivery,
        AcknowledgeDelivery.fromChange(change),
      );
    },
  );
}

@override
void dispose() {
  eventSubscription.cancel();
  super.dispose();
}
```

The stable consumer `id` owns a durable checkpoint. Cancelling unloads the live subscription; reopening resumes from the last committed event. `context.enqueue(...)` atomically commits both the new action and the event checkpoint, so a crash cannot acknowledge an event without retaining the resulting work.

Two APIs should be distinct:

- `events.watch(...)` is an ephemeral stream for display-only reactions.
- `events.consume(id: ..., onEvent: ...)` is a serialized, durable side-effect consumer. Its checkpoint advances only after the asynchronous handler and local transaction succeed.

## Required correctness rules

These are part of the public contract, not implementation details:

- Apply a server row and advance that table's durable checkpoint in the same SQLite transaction.
- Insert an outbox item and apply its optimistic local change in the same SQLite transaction.
- Resume a subscription from the last SQLite-committed checkpoint, not the last event read from the socket.
- Use a stable action UUID as the idempotency key for every retry, including the "server accepted it but the response was lost" case.
- Serialize all SDK cache/outbox writes through one executor. Do not use a shared database flag to suppress triggers.
- Partition the database by server, namespace, and authenticated subject. Stop sync before changing accounts.
- Preserve per-action ordering keys when actions for one conversation or aggregate must execute in order.
- Expose one replaying state object covering `offline`, `catchingUp`, `flushing`, `live`, and `error`, with pending and failed action counts.
- On an expired server cursor, perform a deterministic table rebootstrap rather than silently skipping history.

### `kalam_link` durability boundary

`kalam_link.liveEventsWithAck(...)` is the sync-only transport primitive. It
does not advance effective reconnect progress when an event is read. The Dart
pump applies consumer backpressure, and `kalam_sync` acknowledges each server
batch only after its rows and checkpoint commit in one SQLite transaction.

Existing `liveEvents(..., onCheckpoint: ...)` remains automatic for ordinary
listeners. Its callback must not be used as a durable SQLite cursor because it
runs before application persistence succeeds.

## What is generated

`kalam_sync_generator` should generate only deterministic glue:

- stable module and action registry entries;
- payload encode/decode functions;
- duplicate action-name and unsupported-payload compile errors;
- the action version in the durable key;
- the install adapter used by `kalam.actions.install(module)`.

The annotations themselves remain in `kalam_sync`; only the builder is a separate development package. Retry scheduling, SQLite transactions, action state, socket state, and execution remain runtime responsibilities of `kalam_sync`.

Renaming a Dart field must not rename a persisted action. `@KalamAction(name: ..., version: ...)` is therefore explicit and required for generated durable actions.

### Generation pipeline, one owner for table models

Kalam must not generate a second set of Dart row, insert, or patch models. Drift already generates a full-row data class and a companion for partial inserts/updates for every declared table. The pipeline is:

| Stage | Input | Output | Owns application models? |
| --- | --- | --- | --- |
| `kalam schema gen --languages dart` | KalamDB schema metadata | `.drift` table declarations plus Kalam remote-table/primary-key metadata | No |
| Drift's builder | generated `.drift` declarations | row data classes, companions, typed queries, and local accessors | Yes |
| `kalam_sync_generator` | Kalam metadata, Drift modular output, and application annotations | thin sync adapters, typed `KalamChange<DriftRow>` glue, and custom-action registries/codecs | Only custom action payloads |

The CLI Dart target now emits row classes plus `KalamTableSpec` values from `schema.sql`. Drift `.drift` table declarations remain deferred so generated files compile against `kalam_sync` alone. Schema gen still does not emit competing CRUD payload classes; those stay with `kalam_sync_generator`.

There is also no generated `CreateTodoAction`, `UpdateTodoAction`, and `DeleteTodoAction` for every table. Bidirectional CRUD uses one SDK-internal DML envelope containing the remote table ID, operation, key, changed values, schema fingerprint, and action UUID. The public typed write methods accept Drift-generated data classes or companions.

Custom business actions remain different because their payloads are not necessarily table rows. `SendMessage`, `MarkSeen`, or a future server function may have dedicated argument/result models generated from their own contract.

Generation and subscription are separate decisions:

- CLI include/exclude options choose which bindings exist in the application binary.
- `bidirectional()` and `replicaOnly()` choose which generated tables sync for an authenticated session.
- `events.watch()` or `events.consume()` starts a subscription when a feature needs it.
- Generating a table, event type, or future server function must never open a socket or register a listener by itself.

See [04-generated-bindings-and-future-functions.md](04-generated-bindings-and-future-functions.md) for the proposed generated syntax and the compatibility seam for future KalamDB functions.

## Masky acceptance contract

The API is successful only when Masky can delete its socket, checkpoint, and send-queue infrastructure. Before that migration, `kalam_sync` must prove:

- strict, serialized event application per subscription; a failed event blocks later checkpoint advancement rather than being skipped;
- sparse update merging, explicit insert/update/delete operation types, FILE values, tombstones, and preservation of declared local-only columns;
- a projection adapter for apps that keep domain tables separate from raw synced rows;
- durable custom consumers for flows such as `user_updates -> REST fetch -> local projection`, including retry and dead-letter state;
- atomic local mutation plus custom action enqueue for messages, conversation workflows, reactions, seen state, settings, and notification actions;
- foreground pause/resume and a bounded headless catch-up entry point suitable for an FCM isolate;
- reactive auth refresh, stale-session fencing, and safe user switching;
- sync state that distinguishes transport readiness, catch-up completeness, paused/auth-error state, pending/failed actions, and the last successful commit time.

Masky should retain message previews, unread calculations, contact hydration, notification presentation, and REST payload construction. The SDK should absorb socket ownership, ordering, durable checkpoints, outbox storage, retry, pause/resume, and connection/sync state.

## Lessons from other systems

- [PowerSync client-side integration](https://docs.powersync.com/configuration/app-backend/client-side-integration) atomically records local CRUD and its upload queue, then invokes a central upload hook after writes and reconnection. Kalam should provide the same atomicity and automatic flushing, while supplying built-in KalamDB DML execution.
- [RxDB replication](https://rxdb.info/replication.html) separates pull and push handlers, supports pull-only replication, and treats checkpoints and stable replication identifiers as core concepts. This supports explicit per-table modes and stable consumer IDs.
- [Replicache mutators](https://doc.replicache.dev/tutorial/adding-mutators) package optimistic local work as registered mutations that are queued and later applied authoritatively. Kalam's callable action definitions keep the same useful mental model while also supporting backend-authoritative actions with no optimistic phase.
- [WatermelonDB writers](https://watermelondb.dev/docs/Writers) organize related writes in feature/model methods and mark their transaction boundary explicitly. Kalam action modules follow that organization without coupling durable executors to widgets.
- [Drift DAOs](https://drift.simonbinder.eu/dart_api/daos/) use annotated feature accessors to split a database API into manageable modules. The action-module generator follows this established Dart shape.
- Dart's official [build_runner documentation](https://dart.dev/tools/build_runner) and [`source_gen` documentation](https://pub.dev/documentation/source_gen/latest/) support an optional annotation package plus generated part files; the runtime API must still work without generation.
- [SpaceTimeDB reducers](https://spacetimedb.com/docs/functions/reducers/) show the value of transactional server functions, while its [client binding generator](https://spacetimedb.com/docs/clients/codegen/) generates table, subscription, reducer-call, and callback APIs from one server contract. Kalam should preserve that future path without putting backend functions into frontend-only v1.

## Examples in this folder

- [01-bidirectional-todos.md](01-bidirectional-todos.md): offline CRUD with automatic Kalam DML outbox.
- [02-mixed-replication-modes.md](02-mixed-replication-modes.md): bidirectional and replica-only tables on one connection.
- [03-feature-actions-and-events.md](03-feature-actions-and-events.md): generated action modules, delete-then-enqueue, backend-authoritative enqueue, and a widget-scoped durable event consumer.
- [04-generated-bindings-and-future-functions.md](04-generated-bindings-and-future-functions.md): typed table/change generation now and server-function generation later without changing the frontend action model.
- [05-offline-messaging.md](05-offline-messaging.md): one-conversation messaging with a backend-authoritative replica, optimistic offline rows, custom endpoint actions, and per-row sync state.

## Suggested implementation order

1. Keep explicit-consumer-ack reconnect and SQLite commit-order tests green.
2. Implement the account-partitioned Drift store, table checkpoints, server apply transaction, and sync state.
3. Implement `replicaOnly` download/apply and expired-cursor rebootstrap.
4. Implement bidirectional built-in DML actions with atomic optimistic writes and idempotent retries.
5. Implement manual feature action modules and action-state watches.
6. Implement Dart CLI generation for `.drift` declarations and Kalam table metadata, then generate thin adapters over Drift's row/companion types.
7. Add the `kalam_sync` annotations and optional `kalam_sync_generator` for typed application-action module glue.
8. Implement `events.watch` and durable `events.consume` over the same shared `kalam_link` connection.
9. Add bounded headless catch-up, projection adapters, and sparse-update/delete conformance tests.
10. Migrate Masky one feature at a time: messages, conversations, notifications, event changes, then its REST action outbox.
