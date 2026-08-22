# Generated bindings and future KalamDB functions

Status: the table/change binding design belongs to frontend-only v1. Backend functions are a compatibility target, not part of the current implementation plan.

## Decision

Adding transactional backend functions later is a strong fit for this design. A frontend action is durable **intent** stored in the local outbox; its executor may be either:

- a client-defined HTTP/OpenAPI handler in v1; or
- a generated KalamDB server-function invocation in the future.

The UI should not care which executor is used:

```dart
await deleteTodo(DeleteTodoArgs(todoId: todoId));
```

That call always means “store this action durably.” It must never sometimes execute HTTP immediately and sometimes enqueue, depending on its implementation.

## Terminology

SpaceTimeDB calls transactional database-mutating functions **reducers**. KalamDB should reserve precise terms:

- **Action**: a durable client intent in `kalam_sync`.
- **Transaction function** or **command**: a future atomic server function that reads/writes KalamDB tables. This is equivalent to a SpaceTimeDB reducer.
- **Procedure**: a future server function that can perform external I/O. Its retry and transaction guarantees differ from a database-only command.

The public server namespace can still be `functions`, with a declared `kind` of `transaction` or `procedure`. Calling every function a reducer would be misleading once external side effects are allowed.

## CLI-generated Drift schema and Kalam metadata

The Dart CLI generator should accept selective bindings:

```toml
[schema.targets.dart]
output = "lib/generated/kalam"
include_tables = [
  "app.todos",
  "app.messages",
  "app.message_events",
]

# Reserved for the future backend-functions feature. It remains empty in v1.
include_functions = []
```

For each table, `kalam schema gen` should generate:

- a `.drift` table declaration matching the synced server columns;
- metadata mapping the local Drift table to its remote `TableId`;
- primary/composite key and schema-fingerprint metadata;
- declared local-only columns that server apply must preserve.

It should not generate a second Dart row, insert, patch, or CRUD-action model. Drift's builder consumes the `.drift` declaration and generates the row data class, companion, typed local queries, and accessors. Drift documents these as the two standard table types: a full-row data class and a companion representing partial insert/update values.

After Drift runs, `kalam_sync_generator` emits only a thin adapter referencing those Drift types. With modular Drift generation, the build can order the Kalam builder after Drift and import the independently generated `.drift.dart` library.

The generated adapters keep the mixed-mode connection type-safe:

```dart
import 'package:my_app/generated/kalam/kalam.dart';

final todoSync = AppTables.todos.bidirectional();
final messageSync = AppTables.messages.replicaOnly();

final kalam = await Kalam.open(
  url: serverUrl,
  auth: auth,
  tables: [todoSync, messageSync],
);

final todos = todoSync.bind(kalam);       // writable typed table
final messages = messageSync.bind(kalam); // read-only typed replica

await todos.insert(
  TodosCompanion.insert(
    id: Kalam.id(),
    title: 'Generated and offline-first',
  ),
);

messages.watch().listen(renderMessages);
```

`messageSync.bind(kalam)` should not expose insert/update/delete methods. Choosing `replicaOnly()` should remove those operations at compile time, not merely throw at runtime.

## CRUD does not create three action models per table

`todos.insert`, `todos.update`, and `todos.delete` use Drift values at the public boundary, then serialize into one private envelope:

```dart
final class KalamDmlAction {
  const KalamDmlAction({
    required this.actionId,
    required this.tableId,
    required this.operation,
    required this.key,
    required this.values,
    required this.schemaFingerprint,
  });

  final String actionId;
  final TableId tableId;
  final KalamDmlOperation operation;
  final Map<String, Object?> key;
  final Map<String, Object?> values;
  final String schemaFingerprint;
}
```

This model is internal to `kalam_sync`. It is sufficient for every ordinary table mutation and keeps outbox migrations independent of app-generated type names.

Drift remains the type system and query engine, but writes to a synced table must go through its generated Kalam binding so the local write and outbox item share one transaction:

```dart
await todos.update(
  todoId,
  TodosCompanion(
    completed: const Value(true),
  ),
);
```

Direct reads and local-only writes may use the application database normally. In v1, a direct `appDb.into(appDb.todos).insert(...)` on a synced table is not promised to enter the outbox; reliable capture requires the Kalam binding. This avoids trigger-suppression races while still using Drift's generated types and executor.

## Generated events do not auto-subscribe

The generator creates typed change adapters that reuse Drift's row data class, but a feature decides when to consume them:

```dart
late final KalamEventSubscription subscription;

@override
void initState() {
  super.initState();

  subscription = AppTables.messageEvents.changes.consume(
    kalam,
    id: 'conversation/${widget.conversationId}/message-events-v1',
    where: (event) =>
        event.conversationId.equals(widget.conversationId),
    onEvent: (change, context) async {
      switch (change) {
        case KalamInsert<MessageEvent>(row: final event):
          await context.enqueue(
            messageActions.acknowledgeDelivery,
            AcknowledgeDelivery.fromEvent(event),
          );
        case KalamUpdate<MessageEvent>():
        case KalamDelete<MessageEvent>():
          break;
      }
    },
  );
}

@override
void dispose() {
  subscription.cancel();
  super.dispose();
}
```

This preserves widget/feature ownership. Merely generating `message_events` does not add it to `Kalam.open()` and does not keep an unused live query active.

The generator should not create a queued action for every change event. A change describes something that already happened; an action requests something new. When a change must cause an action, the explicit `context.enqueue(...)` bridge above makes its durability and checkpoint boundary visible.

## Application-defined actions in v1

The `build_runner` generator remains useful for actions that call an existing app backend:

```dart
@KalamActionModule(namespace: 'todos')
class TodoActions with _$TodoActions {
  TodoActions(this.api);

  final TodoApi api;

  @KalamAction(name: 'delete', version: 1)
  late final delete = queuedAction<DeleteTodoArgs>(
    run: (context, args) => api.deleteTodo(
      args.todoId,
      idempotencyKey: context.idempotencyKey,
    ),
  );
}
```

The generated registry and codec use the same `KalamQueuedAction<DeleteTodoArgs>` runtime type that a future CLI-generated server function will use.

## Future generated server functions

If KalamDB later exposes server functions, schema metadata should describe at least:

- stable function name and version;
- `transaction` or `procedure` kind;
- typed input and output;
- authorization requirements;
- idempotency behavior;
- tables the function may change;
- whether it publishes an invocation event;
- result/error schema.

The CLI can then generate a descriptor rather than handwritten Dart execution code:

```dart
final deleteTodo = await kalam.actions.install(
  AppFunctions.deleteTodo.queued(
    optimistic: (local, args) =>
        local.table(AppTables.todos).delete(args.todoId),
  ),
);

final receipt = await deleteTodo(
  DeleteTodoArgs(todoId: todoId),
);

await receipt.queued; // durable locally
await receipt.acked;  // server function committed
await receipt.synced; // affected local replicas reached the commit watermark
```

No `run:` closure is needed because the generated function descriptor is the executor. The server invocation must accept the action UUID as an idempotency key and return its commit watermark. Without those two fields, offline retry can duplicate a successful function and `receipt.synced` cannot know when the local cache has observed its effects.

Users opt into only the functions they use:

```dart
final actions = await kalam.actions.installAll([
  AppFunctions.deleteTodo.queued(),
  AppFunctions.archiveConversation.queued(),
]);
```

An uninstalled generated function has no outbox executor. Generation alone does not register it and does not subscribe to invocation events.

## Pre- and post-action behavior

Generated files must never be edited. User behavior attaches through explicit policy and state APIs:

```dart
final deleteTodo = AppFunctions.deleteTodo.queued(
  optimistic: (local, args) =>
      local.table(AppTables.todos).delete(args.todoId),
);

deleteTodo.watch().listen((state) {
  // queued, running, retryScheduled, acked, synced, or failed
});
```

The lifecycle names need exact semantics:

| Extension point | Guarantee |
| --- | --- |
| `optimistic` / `beforeEnqueue` | Runs inside the same SQLite transaction as outbox insertion |
| `queued` | The action is durable locally |
| `acked` | The backend committed or accepted the action |
| `synced` | Subscribed affected tables have applied the returned commit watermark |
| `failed` | Retry policy classified the action as permanently failed |

Do not expose an ambiguous `beforeSend` hook for business logic: retries may run it multiple times. A UI post-hook should observe the action state. A durable follow-up should be another action enqueued from a durable event consumer so its work and checkpoint commit atomically.

## Subscribing to actions versus changes

These are different subscriptions:

- `action.watch()` observes this client's outbox lifecycle.
- `table.watch()` observes the current local rows.
- `table.changes.consume()` durably processes insert/update/delete events.
- A future `function.invocations.consume()` should exist only if the server explicitly publishes an authorized invocation stream.

The normal source of truth after a server function is still the changed table event, not the reducer/function callback. This keeps direct SQL writes, backend functions, jobs, and other clients on the same synchronization path.

## Compatibility requirement for frontend-only v1

The v1 action runtime should depend on an executor interface rather than directly on an HTTP callback:

```dart
abstract interface class KalamActionExecutor<Input> {
  Future<KalamActionExecutionResult> execute(
    KalamActionContext context,
    Input input,
  );
}
```

The frontend module adapter and future generated server-function adapter both implement this interface. This is the only future seam needed now; KalamDB backend functions themselves remain outside the frontend-only v1 plan.

## Reference model

SpaceTimeDB documents reducers as transactional functions that mutate database state and exposes them to clients automatically: [Reducer overview](https://spacetimedb.com/docs/functions/reducers/). Its generated bindings include table types, reducer callers, subscriptions, local-cache access, and reducer callbacks: [Generating client bindings](https://spacetimedb.com/docs/clients/codegen/).

Drift already generates a full-row data class plus a companion for partial inserts and updates: [Generated table rows](https://drift.simonbinder.eu/dart_api/rows/). Its [modular code generation](https://drift.simonbinder.eu/generation_options/modular/) produces independent libraries and supports ordering other builders after Drift, which is the intended integration point for Kalam's thin adapters.

Kalam should adopt that typed contract generation while retaining its local durable outbox and per-table replication modes.
