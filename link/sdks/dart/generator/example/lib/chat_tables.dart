import 'package:kalam_sync/kalam_sync.dart';

import 'chat_models.dart';

KalamTableSpec<Conversation> chatConversationsSpec(String tableId) {
  return KalamTableSpec<Conversation>(
    tableId: tableId,
    keyColumn: 'id',
    mode: KalamSyncMode.bidirectional,
    keyOf: (row) => row.id,
    encode: (row) => row.toJson(),
    decode: Conversation.fromJson,
  );
}

KalamTableSpec<ChatMessage> chatMessagesSpec(String tableId) {
  return KalamTableSpec<ChatMessage>(
    tableId: tableId,
    keyColumn: 'id',
    mode: KalamSyncMode.replicaOnly,
    keyOf: (row) => row.id,
    encode: (row) => row.toJson(),
    decode: ChatMessage.fromJson,
  );
}
