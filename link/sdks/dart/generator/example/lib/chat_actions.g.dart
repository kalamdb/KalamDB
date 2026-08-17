// GENERATED CODE - DO NOT MODIFY BY HAND

part of 'chat_actions.dart';

// **************************************************************************
// KalamActionPayloadGenerator
// **************************************************************************

SendMessageArgs _$SendMessageArgsFromJson(Map<String, Object?> json) =>
    SendMessageArgs(
      messageId: json['messageId'] as String,
      conversationId: json['conversationId'] as String,
      text: json['text'] as String,
      createdAt: DateTime.parse(json['createdAt'] as String),
      author: json['author'] as String,
    );

Map<String, Object?> _$SendMessageArgsToJson(SendMessageArgs value) => {
  'messageId': value.messageId,
  'conversationId': value.conversationId,
  'text': value.text,
  'createdAt': value.createdAt.toUtc().toIso8601String(),
  'author': value.author,
};

MarkMessageReadArgs _$MarkMessageReadArgsFromJson(Map<String, Object?> json) =>
    MarkMessageReadArgs(
      messageId: json['messageId'] as String,
      conversationId: json['conversationId'] as String,
      readAt: DateTime.parse(json['readAt'] as String),
    );

Map<String, Object?> _$MarkMessageReadArgsToJson(MarkMessageReadArgs value) => {
  'messageId': value.messageId,
  'conversationId': value.conversationId,
  'readAt': value.readAt.toUtc().toIso8601String(),
};

// **************************************************************************
// KalamActionGenerator
// **************************************************************************

List<KalamActionDefinition<dynamic>> chatActionsDefinitions(
  ChatActions module,
) => [
  KalamActionDefinition<SendMessageArgs>(
    key: 'chat.sendMessage',
    version: 1,
    codec: KalamActionCodec(
      encode: _$SendMessageArgsToJson,
      decode: _$SendMessageArgsFromJson,
    ),
    execute: module.sendMessage,
  ),
  KalamActionDefinition<MarkMessageReadArgs>(
    key: 'chat.markMsgRead',
    version: 1,
    codec: KalamActionCodec(
      encode: _$MarkMessageReadArgsToJson,
      decode: _$MarkMessageReadArgsFromJson,
    ),
    execute: module.markMsgRead,
  ),
];

final class ChatActionsQueue {
  const ChatActionsQueue(this._runner);

  final KalamActionRunner _runner;

  Future<KalamActionRecord> sendMessage(
    SendMessageArgs payload, {
    String? actionId,
    String? orderingKey,
    KalamOptimisticMutation? optimistic,
  }) => _runner.enqueue(
    actionKey: 'chat.sendMessage',
    actionId: actionId ?? Kalam.id(),
    payload: payload,
    orderingKey: orderingKey,
    optimistic: optimistic,
  );

  Future<KalamActionRecord> markMsgRead(
    MarkMessageReadArgs payload, {
    String? actionId,
    String? orderingKey,
    KalamOptimisticMutation? optimistic,
  }) => _runner.enqueue(
    actionKey: 'chat.markMsgRead',
    actionId: actionId ?? Kalam.id(),
    payload: payload,
    orderingKey: orderingKey,
    optimistic: optimistic,
  );
}
