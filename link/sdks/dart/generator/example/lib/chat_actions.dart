import 'package:kalam_sync/kalam_sync.dart';

part 'chat_actions.g.dart';

/// Backend calls used by generated chat actions.
abstract interface class ChatActionApi {
  Future<void> persistMessage(
    SendMessageArgs payload, {
    required String idempotencyKey,
  });

  Future<void> markDelivered(
    SendMessageArgs payload, {
    required String idempotencyKey,
  });

  Future<void> markRead(
    MarkMessageReadArgs payload, {
    required String idempotencyKey,
  });
}

@KalamActionPayload()
final class SendMessageArgs {
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

@KalamActionPayload()
final class MarkMessageReadArgs {
  const MarkMessageReadArgs({
    required this.messageId,
    required this.conversationId,
    required this.readAt,
  });

  final String messageId;
  final String conversationId;
  final DateTime readAt;
}

@KalamActionModule(namespace: 'chat')
final class ChatActions {
  const ChatActions(this.api);

  final ChatActionApi api;

  /// Persists the user message, then marks it delivered on a second endpoint.
  ///
  /// Named steps survive retry and process restart. Both endpoints still honor
  /// the supplied idempotency key because a response can be lost after commit.
  @KalamAction(name: 'sendMessage')
  Future<void> sendMessage(
    KalamActionContext context,
    SendMessageArgs payload,
  ) async {
    await context.step<bool>(
      'persist',
      run: (idempotencyKey) async {
        await api.persistMessage(payload, idempotencyKey: idempotencyKey);
        return true;
      },
      encode: (value) => value,
      decode: (value) => value == true,
    );
    await context.step<bool>(
      'deliver',
      run: (idempotencyKey) async {
        await api.markDelivered(payload, idempotencyKey: idempotencyKey);
        return true;
      },
      encode: (value) => value,
      decode: (value) => value == true,
    );
  }

  @KalamAction(name: 'markMsgRead')
  Future<void> markMsgRead(
    KalamActionContext context,
    MarkMessageReadArgs payload,
  ) {
    return context.step<bool>(
      'read',
      run: (idempotencyKey) async {
        await api.markRead(payload, idempotencyKey: idempotencyKey);
        return true;
      },
      encode: (value) => value,
      decode: (value) => value == true,
    );
  }
}
