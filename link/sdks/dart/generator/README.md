# kalam_sync_generator

Code generation for [`kalam_sync`](https://pub.dev/packages/kalam_sync) durable actions.

Annotate an immutable payload and executor module. `dart run build_runner build` writes:

- a JSON codec for each payload
- namespaced `KalamActionDefinition` values
- a typed queue with offline enqueue methods

Drift still owns table row types. Direct create/update/delete uses the runtime's generic DML envelope — this generator does not emit extra CRUD models per table.

## Install

```yaml
dev_dependencies:
  build_runner: ^2.15.1
  kalam_sync_generator: ^0.6.0-rc.0
```

## Get started

```dart
@KalamActionPayload()
class SendMessageArgs {
  const SendMessageArgs({required this.messageId, required this.text});
  final String messageId;
  final String text;
}

@KalamActionModule(namespace: 'chat')
class ChatActions {
  ChatActions(this.api);
  final ChatActionApi api;

  @KalamAction(name: 'sendMessage')
  Future<void> sendMessage(
    KalamActionContext context,
    SendMessageArgs args,
  ) {
    return context.step<bool>(
      'persist',
      run: (key) async {
        await api.persistMessage(args, idempotencyKey: key);
        return true;
      },
      encode: (value) => value,
      decode: (value) => value == true,
    );
  }
}
```

```bash
dart run build_runner build --delete-conflicting-outputs
```

Then open Kalam with the generated definitions and enqueue through the typed queue:

```dart
final kalam = await Kalam.open(
  url: serverUrl,
  subject: userId,
  actionDefinitions: chatActionsDefinitions(ChatActions(api)),
);

await ChatActionsQueue(kalam.actions).sendMessage(
  SendMessageArgs(messageId: Kalam.id(), text: 'hello'),
  orderingKey: conversationId,
  optimistic: messages.optimisticInsert(message),
);
```

Full walkthrough: [Offline actions](https://kalamdb.org/docs/dart-sdk/sync-actions).

The canonical generated module lives in [`example/`](example). The complete chat app is in [`../sync/example`](../sync/example).
