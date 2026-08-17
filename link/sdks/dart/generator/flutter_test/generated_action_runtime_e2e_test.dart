import 'package:drift/native.dart';
import 'package:kalam_sync/drift.dart' hide isNull;
import 'package:kalam_sync/kalam_sync.dart';
import 'package:kalam_sync_generator_example/chat_actions.dart';
import 'package:test/test.dart';

final class _RecordingApi implements ChatActionApi {
  final persistKeys = <String>[];
  final deliverKeys = <String>[];
  final readKeys = <String>[];
  SendMessageArgs? sent;
  MarkMessageReadArgs? read;

  @override
  Future<void> persistMessage(
    SendMessageArgs payload, {
    required String idempotencyKey,
  }) async {
    persistKeys.add(idempotencyKey);
    sent = payload;
  }

  @override
  Future<void> markDelivered(
    SendMessageArgs payload, {
    required String idempotencyKey,
  }) async {
    deliverKeys.add(idempotencyKey);
  }

  @override
  Future<void> markRead(
    MarkMessageReadArgs payload, {
    required String idempotencyKey,
  }) async {
    readKeys.add(idempotencyKey);
    read = payload;
  }
}

void main() {
  test(
    'generated chat queue persists send, multi-step deliver, and mark-read',
    () async {
      final database = KalamSyncDatabase(NativeDatabase.memory());
      addTearDown(database.close);
      final store = KalamSyncStore(database);
      final api = _RecordingApi();
      final runner = KalamActionRunner(
        accountKey: 'server/user-a',
        store: store,
        registry: KalamActionRegistry.of(
          chatActionsDefinitions(ChatActions(api)),
        ),
      );
      final queue = ChatActionsQueue(runner);
      final createdAt = DateTime.utc(2026, 8, 17, 10, 30);
      final readAt = DateTime.utc(2026, 8, 17, 10, 31);

      await queue.sendMessage(
        SendMessageArgs(
          messageId: 'message-1',
          conversationId: 'conversation-1',
          text: 'hello offline',
          createdAt: createdAt,
          author: 'ada',
        ),
        actionId: 'send-message-1',
        orderingKey: 'conversation-1',
      );
      expect(api.sent, isNull, reason: 'enqueue must not execute while offline');
      expect(await runner.flush(), 1);
      expect(api.sent?.messageId, 'message-1');
      expect(api.sent?.text, 'hello offline');
      expect(api.sent?.createdAt, createdAt);
      expect(api.persistKeys, ['send-message-1/persist']);
      expect(api.deliverKeys, ['send-message-1/deliver']);
      expect(
        (await store.readAction('send-message-1'))?.status,
        KalamActionStatus.succeeded,
      );

      await queue.markMsgRead(
        MarkMessageReadArgs(
          messageId: 'message-1',
          conversationId: 'conversation-1',
          readAt: readAt,
        ),
        actionId: 'read-message-1',
        orderingKey: 'conversation-1',
      );
      expect(await runner.flush(), 1);
      expect(api.read?.messageId, 'message-1');
      expect(api.readKeys, ['read-message-1/read']);
    },
  );
}
