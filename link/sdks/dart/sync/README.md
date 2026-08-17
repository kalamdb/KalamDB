# kalam_sync

Local-first Flutter synchronization for KalamDB. The package adds one Drift
cache, durable resume checkpoints, optimistic row state, an offline action
outbox, retries, lifecycle handling, and widget-scoped event consumers on top
of the existing shared `kalam_link` connection.

## Open one account-scoped cache

```dart
final kalam = await Kalam.open(
  url: 'https://db.example.com',
  subject: signedInUser.id,
  authProvider: () async => Auth.jwt(await tokens.freshAccessToken()),
  actionDefinitions: messageActionsDefinitions(MessageActions(api)),
);

runApp(KalamScope(kalam: kalam, child: const App()));
```

`subject` is required. The server URL, namespace, and authenticated subject
form the local database identity, so one user's cached rows and queued actions
cannot be opened or flushed as another user.

The WebSocket is lazy. Opening the cache does not wait for a network
connection; the first active consumer or queued action starts the existing
`kalam_link` socket.

## Bidirectional todos

`kalam schema gen --languages dart` writes row codecs and `KalamTableSpec`
values (for example `KalamTables.todos`). Kalam only needs that table codec
and does not create a second todo model:

```dart
final todos = kalam.table(
  KalamTableSpec<Todo>(
    tableId: 'app.todos',
    keyColumn: 'id',
    mode: KalamSyncMode.bidirectional,
    keyOf: (todo) => todo.id,
    encode: (todo) => todo.toJson(),
    decode: Todo.fromJson,
  ),
);

late final KalamSyncSubscription todoSync;

Future<void> startTodos() async {
  todoSync = await kalam.subscribe(
    todos.consumer(sql: 'SELECT * FROM app.todos', batchSize: 250),
  );
}

Future<void> addTodo(String title) {
  return todos.insert(
    Todo(id: Kalam.id(), title: title, completed: false),
    actionId: Kalam.id(),
  );
}
```

`insert`, `update`, and `delete` change the local row and enqueue one generic
Kalam DML action in the same SQLite transaction. `todos.watch()` is always
local and continues to work offline. Use `watchWithSyncState()` when the UI
needs pending, retry, failure, or awaiting-server-echo state beside each row.

## Subscribe inside a widget

Consumers are not registered in one global `events: []` list. A widget or
feature starts only the query it needs and cancels it when unloaded:

```dart
class ConversationState extends State<Conversation> {
  late final KalamTableBinding<Message> messages;
  KalamSyncSubscription? sync;

  @override
  void initState() {
    super.initState();
    final kalam = KalamScope.read(context);
    messages = kalam.table(AppTables.messages);
    final conversationId = widget.conversationId;
    if (!RegExp(r'^[A-Za-z0-9_-]+$').hasMatch(conversationId)) {
      throw ArgumentError.value(conversationId, 'conversationId');
    }
    kalam
        .subscribe(messages.consumer(
          sql: "SELECT * FROM app.messages "
              "WHERE conversation_id = '$conversationId'",
        ))
        .then((value) {
          if (mounted) {
            sync = value;
          } else {
            unawaited(value.cancel());
          }
        });
  }

  @override
  void dispose() {
    sync?.cancel();
    super.dispose();
  }
}
```

`liveEvents` does not currently accept bind parameters, so validate any value
before placing it in a subscription query. The example permits only a narrow
identifier alphabet; generated query helpers should apply the server type's
exact validation rule.

## Backend-authoritative messages with offline actions

For a custom HTTP workflow, make the table `replicaOnly`. The backend remains
authoritative, while the action owns an optimistic overlay:

```dart
@KalamActionPayload()
class SendMessageArgs {
  const SendMessageArgs({
    required this.messageId,
    required this.conversationId,
    required this.text,
    required this.createdAt,
    required this.author,
  });
  final String messageId;
  final String conversationId;
  final String text;
  final DateTime createdAt;
  final String author;
}

@KalamActionModule(namespace: 'chat')
class ChatActions {
  ChatActions(this.api);
  final ChatActionApi api;

  @KalamAction(name: 'sendMessage')
  Future<void> sendMessage(
    KalamActionContext context,
    SendMessageArgs args,
  ) async {
    await context.step<bool>(
      'persist',
      run: (key) async {
        await api.persistMessage(args, idempotencyKey: key);
        return true;
      },
      encode: (value) => value,
      decode: (value) => value == true,
    );
    await context.step<bool>(
      'deliver',
      run: (key) async {
        await api.markDelivered(args, idempotencyKey: key);
        return true;
      },
      encode: (value) => value,
      decode: (value) => value == true,
    );
  }
}
```

Run `dart run build_runner build` in `link/sdks/dart/generator/example`.
`kalam_sync_generator` generates the JSON codec, `chat.sendMessage` definition,
and `ChatActionsQueue.sendMessage()`:

```dart
final messageRows = kalam.table(chatMessagesSpec('chat.messages'));
final actions = ChatActionsQueue(kalam.actions);
final message = ChatMessage(
  id: Kalam.id(),
  conversationId: conversationId,
  role: ChatMessageRole.user,
  author: userId,
  text: text,
  status: ChatDeliveryStatus.pending,
  createdAt: DateTime.now().toUtc(),
);

await actions.sendMessage(
  SendMessageArgs(
    messageId: message.id,
    conversationId: conversationId,
    text: message.text,
    createdAt: message.createdAt,
    author: message.author,
  ),
  orderingKey: conversationId,
  optimistic: messageRows.optimisticInsert(message),
);
```

The generated queue method commits the optimistic row, sidecar sync state, and
serialized action together. On connectivity, the executor calls the backend
with the stable action UUID. The row becomes `synced` only after the backend's
KalamDB write arrives and is committed locally.

For multi-endpoint workflows, use `context.step(...)`. A completed named step
is persisted and reused after retry or process restart. Every remote endpoint
must still honor the supplied idempotency key because a response can be lost
after the server commits.

## Generation boundary

- Drift owns generated table row/companion types.
- `kalam_sync_generator` owns action payload codecs, definitions, and queues.
- Kalam uses one generic internal envelope for direct create/update/delete;
  it does not generate three extra action model classes for every table.
- The complete offline-first chat app, REST engine, schema, and five live-server
use-case tests live in [`example/`](example). Conversations use bidirectional
DML; messages stay `replicaOnly` and sync through generated `sendMessage` /
`markMsgRead` actions.

## Correctness guarantees

- Local mirror row + outbox enqueue are one Drift transaction.
- Applied server row + durable sequence checkpoint are one Drift transaction.
- Server deliveries are acknowledged only after that transaction commits;
  the next delivery is not pulled before acknowledgement.
- Duplicate or older events are ignored.
- Actions, retry metadata, named steps, optimistic rows, and checkpoints
  survive process restart.
- Manual lifecycle pause/resume cancels and reopens subscriptions from the
  SQLite-committed checkpoint.

`kalam_sync` uses the additive `kalam_link.liveEventsWithAck` API. Existing
`liveEvents` callers keep automatic progress, while sync subscriptions resume
from the SQLite-committed cursor after reconnect or process restart.
