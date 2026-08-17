# Example 3: feature actions and widget events

This example shows where registered actions live, how a local method becomes durable work, and how a widget can mount a durable event consumer without putting all event logic in `Kalam.open()`.

## Define actions beside the feature

```dart
// lib/features/messages/message_actions.dart
import 'package:kalam_sync/kalam_sync.dart';

part 'message_actions.kalam.dart';

@KalamActionPayload()
class DeleteMessage {
  const DeleteMessage({required this.messageId});

  final String messageId;
}

@KalamActionPayload()
class DeleteTodoViaWorkflow {
  const DeleteTodoViaWorkflow({required this.todoId});

  final String todoId;
}

@KalamActionPayload()
class AcknowledgeDelivery {
  const AcknowledgeDelivery({
    required this.messageId,
    required this.eventId,
  });

  final String messageId;
  final String eventId;

  factory AcknowledgeDelivery.fromChange(KalamChange change) {
    return AcknowledgeDelivery(
      messageId: change.row.stringValue('message_id'),
      eventId: change.row.stringValue('id'),
    );
  }
}

@KalamActionModule(namespace: 'messages')
class MessageActions with _$MessageActions {
  MessageActions(this.api);

  final MessageApi api;

  // Backend-authoritative: enqueue only. app.messages is replicaOnly.
  @KalamAction(name: 'delete', version: 1)
  late final delete = queuedAction<DeleteMessage>(
    retry: const KalamRetry.exponential(
      maxAttempts: 12,
      maxDelay: Duration(minutes: 5),
    ),
    run: (context, command) => api.deleteMessage(
      command.messageId,
      idempotencyKey: context.idempotencyKey,
    ),
  );

  // Custom workflow: delete locally and enqueue exactly one custom action.
  // app.todos is bidirectional, but writes through this optimistic context
  // are captured by this action instead of creating a second DML action.
  @KalamAction(name: 'delete-todo-workflow', version: 1)
  late final deleteTodoViaWorkflow =
      queuedAction<DeleteTodoViaWorkflow>(
    optimistic: (local, command) =>
        local.table('app.todos').delete(command.todoId),
    run: (context, command) => api.deleteTodoViaWorkflow(
      command.todoId,
      idempotencyKey: context.idempotencyKey,
    ),
  );

  @KalamAction(name: 'acknowledge-delivery', version: 1)
  late final acknowledgeDelivery =
      queuedAction<AcknowledgeDelivery>(
    orderingKey: (command) => command.messageId,
    run: (context, command) => api.acknowledgeDelivery(
      messageId: command.messageId,
      eventId: command.eventId,
      idempotencyKey: context.idempotencyKey,
    ),
  );
}
```

The action object owns both phases when both exist:

- `optimistic` runs only while the outbox record is being created;
- `run` runs only from the outbox executor;
- calling the action always means enqueue;
- the generator owns payload serialization and registration.

This avoids a separate switch statement, action-name constants, JSON mapping file, retry service, and per-feature connectivity listener.

## Install at authenticated-session scope

```dart
class AppSession {
  AppSession({
    required this.kalam,
    required this.messageActions,
  });

  final Kalam kalam;
  final MessageActions messageActions;

  static Future<AppSession> start({
    required Auth auth,
    required MessageApi messageApi,
  }) async {
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
      ],
    );

    final messageActions = await kalam.actions.install(
      MessageActions(messageApi),
    );

    return AppSession(
      kalam: kalam,
      messageActions: messageActions,
    );
  }

  Future<void> close() => kalam.close();
}
```

Larger apps can install each module in its feature provider and expose it through Riverpod, GetIt, or the app's existing dependency injection. The requirement is lifetime, not a particular service-locator pattern: executors remain installed while their session outbox may flush.

## Local methods stay one line

```dart
class MessageRepository {
  MessageRepository(this.actions);

  final MessageActions actions;

  Future<void> deleteMessage(String messageId) {
    return actions.delete(
      DeleteMessage(messageId: messageId),
    );
  }
}
```

Calling this offline succeeds after the durable action is stored. The repository does not check the connection or schedule a retry.

The custom optimistic workflow is equally small at the call site:

```dart
await messageActions.deleteTodoViaWorkflow(
  DeleteTodoViaWorkflow(todoId: todoId),
);
```

The local delete and action insert either both commit or both roll back.

## Mount a durable consumer in one widget

```dart
class ConversationScreenState extends State<ConversationScreen> {
  late final KalamEventSubscription deliveryEvents;

  Kalam get kalam => widget.session.kalam;
  MessageActions get actions => widget.session.messageActions;

  @override
  void initState() {
    super.initState();

    deliveryEvents = kalam.events.consume(
      id: 'conversation/${widget.conversationId}/delivery-events-v1',
      query: '''
        SELECT *
        FROM app.message_events
        WHERE conversation_id = $1
      ''',
      params: [widget.conversationId],
      onEvent: (change, context) async {
        if (change case KalamInsert()) {
          await context.enqueue(
            actions.acknowledgeDelivery,
            AcknowledgeDelivery.fromChange(change),
          );
        }
      },
    );
  }

  @override
  void dispose() {
    deliveryEvents.cancel();
    super.dispose();
  }
}
```

The event consumer is feature-scoped, but its checkpoint is durable. If the widget unloads, the live query is removed. If it opens again, the same consumer ID resumes from the last event whose handler transaction committed.

`context.enqueue(...)` is stronger than calling an external API inside `onEvent`: the action insert and event checkpoint commit together. The action executor may later retry using its stable idempotency key.

If a callback only changes in-memory UI and does not need durable replay, use the lighter stream:

```dart
final stream = kalam.events.watch(
  query: 'SELECT * FROM app.presence WHERE room_id = $1',
  params: [roomId],
);
```

## Runtime action state

Actions should be observable without reading private outbox tables. A typed definition exposes its own states:

```dart
messageActions.delete.watch(
  key: (command) => command.messageId,
).listen((state) {
  // queued, running, retryScheduled, succeeded, or failed
});
```

The session also exposes one merged stream containing built-in table mutations and custom feature actions:

```dart
kalam.actions.watch().listen((change) {
  // Added, started, retryScheduled, succeeded, failed, or removed.
  // change.kind distinguishes builtInDml from customAction.
});
```

This is the supported outbox subscription API. The physical Drift outbox tables remain private so their schema can evolve without breaking applications.

A successful backend response means the action finished. It does not mean a `replicaOnly` row has already changed locally; that remains tied to the inbound Kalam event and table checkpoint.
