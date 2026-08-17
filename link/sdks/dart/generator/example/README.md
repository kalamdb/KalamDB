# Chat action module

This package is the source of truth for the offline-first chat actions used by
`kalam_sync` and `kalam_link`.

It generates:

- `chat.sendMessage` — two durable steps (`persist`, then `deliver`) so a
  multi-endpoint workflow survives retry and restart
- `chat.markMsgRead` — a side-effect action that updates seen state on the
  backend-authoritative messages replica

The runnable Flutter app, REST engine, and schema live in
[`link/sdks/dart/sync/example`](../../sync/example).

## Generate

```bash
cd link/sdks/dart/generator/example
flutter pub get
dart run build_runner build --delete-conflicting-outputs
```

That writes `lib/chat_actions.g.dart`, which the Flutter example and e2e tests
import as `package:kalam_sync_generator_example/chat_actions.dart`.

## What the generated API looks like

```dart
final kalam = await Kalam.open(
  url: kalamUrl,
  subject: userId,
  actionDefinitions: chatActionsDefinitions(ChatActions(api)),
);

final messages = kalam.table(chatMessagesSpec('$namespace.messages'));
final actions = ChatActionsQueue(kalam.actions);

await actions.sendMessage(
  SendMessageArgs(
    messageId: Kalam.id(),
    conversationId: conversationId,
    text: text,
    createdAt: DateTime.now().toUtc(),
    author: userId,
  ),
  orderingKey: conversationId,
  optimistic: messages.optimisticInsert(pendingRow),
);
```

`replicaOnly` messages cannot be inserted through `table.insert`. Conversations
use `KalamSyncMode.bidirectional` in the Flutter example.

## Run the complete example

See [`../../sync/example/README.md`](../../sync/example/README.md).
