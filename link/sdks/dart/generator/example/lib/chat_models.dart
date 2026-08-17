/// Shared chat domain models used by generated actions, the Flutter example,
/// and the live-server e2e suite.
library;

DateTime parseChatTimestamp(Object? value) {
  if (value == null) {
    throw const FormatException('missing timestamp');
  }
  if (value is DateTime) return value.toUtc();
  if (value is int) return _fromNumericTimestamp(value);
  if (value is num) return _fromNumericTimestamp(value.toInt());
  if (value is String) {
    final numeric = int.tryParse(value) ?? double.tryParse(value)?.toInt();
    if (numeric != null) return _fromNumericTimestamp(numeric);
    return DateTime.parse(value).toUtc();
  }
  throw FormatException('Cannot parse timestamp: $value');
}

DateTime? parseOptionalChatTimestamp(Object? value) {
  if (value == null) return null;
  if (value is String && value.trim().isEmpty) return null;
  return parseChatTimestamp(value);
}

DateTime _fromNumericTimestamp(int value) {
  const millisecondThreshold = 9999999999999;
  if (value.abs() > millisecondThreshold) {
    return DateTime.fromMicrosecondsSinceEpoch(value, isUtc: true);
  }
  return DateTime.fromMillisecondsSinceEpoch(value, isUtc: true);
}

String encodeChatTimestamp(DateTime value) => value.toUtc().toIso8601String();

enum ChatMessageRole {
  user,
  assistant;

  static ChatMessageRole parse(Object? value) {
    final name = value?.toString() ?? 'user';
    return ChatMessageRole.values.firstWhere(
      (role) => role.name == name,
      orElse: () => ChatMessageRole.user,
    );
  }
}

enum ChatDeliveryStatus {
  pending,
  sent,
  delivered,
  read;

  static ChatDeliveryStatus parse(Object? value) {
    final name = value?.toString() ?? 'sent';
    return ChatDeliveryStatus.values.firstWhere(
      (status) => status.name == name,
      orElse: () => ChatDeliveryStatus.sent,
    );
  }
}

final class Conversation {
  const Conversation({
    required this.id,
    required this.title,
    required this.createdAt,
    required this.updatedAt,
  });

  final String id;
  final String title;
  final DateTime createdAt;
  final DateTime updatedAt;

  Conversation copyWith({String? title, DateTime? updatedAt}) {
    return Conversation(
      id: id,
      title: title ?? this.title,
      createdAt: createdAt,
      updatedAt: updatedAt ?? this.updatedAt,
    );
  }

  Map<String, Object?> toJson() => {
    'id': id,
    'title': title,
    'created_at': encodeChatTimestamp(createdAt),
    'updated_at': encodeChatTimestamp(updatedAt),
  };

  factory Conversation.fromJson(Map<String, Object?> json) {
    return Conversation(
      id: json['id']!.toString(),
      title: json['title']!.toString(),
      createdAt: parseChatTimestamp(json['created_at']),
      updatedAt: parseChatTimestamp(json['updated_at']),
    );
  }
}

final class ChatMessage {
  const ChatMessage({
    required this.id,
    required this.conversationId,
    required this.role,
    required this.author,
    required this.text,
    required this.status,
    required this.createdAt,
    this.deliveredAt,
    this.readAt,
  });

  final String id;
  final String conversationId;
  final ChatMessageRole role;
  final String author;
  final String text;
  final ChatDeliveryStatus status;
  final DateTime createdAt;
  final DateTime? deliveredAt;
  final DateTime? readAt;

  bool get isAssistant => role == ChatMessageRole.assistant;

  ChatMessage copyWith({
    ChatDeliveryStatus? status,
    DateTime? deliveredAt,
    DateTime? readAt,
  }) {
    return ChatMessage(
      id: id,
      conversationId: conversationId,
      role: role,
      author: author,
      text: text,
      status: status ?? this.status,
      createdAt: createdAt,
      deliveredAt: deliveredAt ?? this.deliveredAt,
      readAt: readAt ?? this.readAt,
    );
  }

  Map<String, Object?> toJson() => {
    'id': id,
    'conversation_id': conversationId,
    'role': role.name,
    'author': author,
    'text': text,
    'status': status.name,
    'created_at': encodeChatTimestamp(createdAt),
    'delivered_at': deliveredAt == null
        ? null
        : encodeChatTimestamp(deliveredAt!),
    'read_at': readAt == null ? null : encodeChatTimestamp(readAt!),
  };

  factory ChatMessage.fromJson(Map<String, Object?> json) {
    return ChatMessage(
      id: json['id']!.toString(),
      conversationId: (json['conversation_id'] ?? json['conversationId'])!
          .toString(),
      role: ChatMessageRole.parse(json['role']),
      author: json['author']!.toString(),
      text: json['text']!.toString(),
      status: ChatDeliveryStatus.parse(json['status']),
      createdAt: parseChatTimestamp(json['created_at'] ?? json['createdAt']),
      deliveredAt: parseOptionalChatTimestamp(
        json['delivered_at'] ?? json['deliveredAt'],
      ),
      readAt: parseOptionalChatTimestamp(json['read_at'] ?? json['readAt']),
    );
  }
}
