import 'dart:convert';
import 'dart:io';

import 'chat_actions.dart';

final class ChatHttpApi implements ChatActionApi {
  ChatHttpApi({required this.baseUrl, HttpClient? client})
    : _client = (client ?? HttpClient())
        ..connectionTimeout = const Duration(seconds: 10)
        ..idleTimeout = const Duration(seconds: 15);

  final Uri baseUrl;
  final HttpClient _client;

  @override
  Future<void> persistMessage(
    SendMessageArgs payload, {
    required String idempotencyKey,
  }) {
    return post('/v1/messages', {
      'messageId': payload.messageId,
      'conversationId': payload.conversationId,
      'text': payload.text,
      'createdAt': payload.createdAt.toUtc().toIso8601String(),
      'author': payload.author,
      'idempotencyKey': idempotencyKey,
    });
  }

  @override
  Future<void> markDelivered(
    SendMessageArgs payload, {
    required String idempotencyKey,
  }) {
    return post(
      '/v1/messages/${Uri.encodeComponent(payload.messageId)}/deliver',
      {
        'conversationId': payload.conversationId,
        'idempotencyKey': idempotencyKey,
      },
    );
  }

  @override
  Future<void> markRead(
    MarkMessageReadArgs payload, {
    required String idempotencyKey,
  }) {
    return post('/v1/messages/${Uri.encodeComponent(payload.messageId)}/read', {
      'conversationId': payload.conversationId,
      'readAt': payload.readAt.toUtc().toIso8601String(),
      'idempotencyKey': idempotencyKey,
    });
  }

  Future<void> post(String path, Map<String, Object?> body) async {
    final request = await _client.postUrl(baseUrl.resolve(path));
    request.headers.contentType = ContentType.json;
    request.write(jsonEncode(body));
    final response = await request.close().timeout(
      const Duration(seconds: 15),
    );
    final text = await utf8.decodeStream(response);
    if (response.statusCode < 200 || response.statusCode >= 300) {
      throw HttpException('Chat backend HTTP ${response.statusCode}: $text');
    }
  }

  void close() => _client.close(force: true);
}
