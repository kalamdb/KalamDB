/// Chat example with conversations, messages, and a typing stream table.
// ignore_for_file: avoid_print
library;

import 'dart:async';

import 'package:kalam_link/kalam_link.dart';

import '../support.dart';

Future<void> main() async {
  await runChatAppExample();
}

Future<void> runChatAppExample({
  String namespace = 'sdk_chat_demo',
  String conversationId = 'project-alpha',
}) async {
  final conversationsTable = '$namespace.conversations';
  final messagesTable = '$namespace.messages';
  final typingTable = '$namespace.typing';
  final quotedConversationId = sqlStringLiteral(conversationId);

  final session = await connectAdminSession();

  try {
    await queryOrThrow(
      session.client,
      'CREATE NAMESPACE IF NOT EXISTS $namespace',
    );
    await queryOrThrow(session.client, 'DROP TABLE IF EXISTS $typingTable');
    await queryOrThrow(session.client, 'DROP TABLE IF EXISTS $messagesTable');
    await queryOrThrow(
      session.client,
      'DROP TABLE IF EXISTS $conversationsTable',
    );

    await queryOrThrow(
      session.client,
      'CREATE TABLE $conversationsTable ('
      'id TEXT PRIMARY KEY, '
      'title TEXT NOT NULL, '
      'summary TEXT NOT NULL DEFAULT \'\', '
      'updated_at TIMESTAMP NOT NULL DEFAULT NOW()'
      ')',
    );
    await queryOrThrow(
      session.client,
      'CREATE TABLE $messagesTable ('
      'id BIGINT PRIMARY KEY, '
      'conversation_id TEXT NOT NULL, '
      'role TEXT NOT NULL, '
      'body TEXT NOT NULL, '
      'status TEXT NOT NULL DEFAULT \'sent\', '
      'created_at TIMESTAMP NOT NULL DEFAULT NOW()'
      ')',
    );
    await queryOrThrow(
      session.client,
      'CREATE STREAM TABLE $typingTable ('
      'id TEXT PRIMARY KEY, '
      'conversation_id TEXT NOT NULL, '
      'actor TEXT NOT NULL, '
      'status TEXT NOT NULL, '
      'token TEXT NOT NULL DEFAULT \'\', '
      'created_at TIMESTAMP NOT NULL DEFAULT NOW()'
      ') WITH (TTL_SECONDS = 30)',
    );

    final latestConversations = <Map<String, KalamCellValue>>[];
    final latestMessages = <Map<String, KalamCellValue>>[];
    final latestTyping = <Map<String, KalamCellValue>>[];

    final conversationsSub = session.client
        .liveTable<Map<String, KalamCellValue>>(
      conversationsTable,
      mapRow: (row) => row,
      limit: 20,
    )
        .listen((rows) {
      latestConversations
        ..clear()
        ..addAll(rows);
      print('conversations: ${rows.length}');
    });

    final messagesSub = session.client
        .live<Map<String, KalamCellValue>>(
      'SELECT id, conversation_id, role, body, status, created_at '
      'FROM $messagesTable '
      'WHERE conversation_id = $quotedConversationId',
      mapRow: (row) => row,
      limit: 50,
    )
        .listen((rows) {
      latestMessages
        ..clear()
        ..addAll(rows);
      print('messages: ${rows.length}');
    });

    final typingSub = session.client
        .live<Map<String, KalamCellValue>>(
      'SELECT id, conversation_id, actor, status, token, created_at '
      'FROM $typingTable '
      'WHERE conversation_id = $quotedConversationId',
      mapRow: (row) => row,
      limit: 20,
    )
        .listen((rows) {
      latestTyping
        ..clear()
        ..addAll(rows);
      print('typing events: ${rows.length}');
    });

    try {
      await queryOrThrow(
        session.client,
        'INSERT INTO $conversationsTable (id, title, summary) '
        'VALUES (\$1, \$2, \$3)',
        params: [
          conversationId,
          'Project Alpha',
          'Conversation with streaming typing indicators',
        ],
      );
      await queryOrThrow(
        session.client,
        'INSERT INTO $messagesTable '
        '(id, conversation_id, role, body, status) '
        'VALUES (\$1, \$2, \$3, \$4, \$5)',
        params: [
          1001,
          conversationId,
          'user',
          'Can you summarize the deployment checklist?',
          'sent',
        ],
      );
      await queryOrThrow(
        session.client,
        'INSERT INTO $typingTable '
        '(id, conversation_id, actor, status, token) '
        'VALUES (\$1, \$2, \$3, \$4, \$5)',
        params: [
          'typing-1',
          conversationId,
          'assistant',
          'typing',
          'Reviewing recent schema changes...',
        ],
      );
      await queryOrThrow(
        session.client,
        'INSERT INTO $messagesTable '
        '(id, conversation_id, role, body, status) '
        'VALUES (\$1, \$2, \$3, \$4, \$5)',
        params: [
          1002,
          conversationId,
          'assistant',
          'Checklist ready: verify auth bootstrap, run smoke tests, and confirm live updates.',
          'complete',
        ],
      );

      await waitForCondition(
        () => latestConversations.any(
          (row) => row['id']?.asString() == conversationId,
        ),
      );
      await waitForCondition(() => latestMessages.length >= 2);
      await waitForCondition(
        () => latestTyping.any(
          (row) => row['status']?.asString() == 'typing',
        ),
      );

      final history = await queryOrThrow(
        session.client,
        'SELECT id, role, body, status '
        'FROM $messagesTable '
        'WHERE conversation_id = \$1 '
        'ORDER BY id',
        params: [conversationId],
      );
      print('Message history:');
      print(describeRows(history.rows));

      print('Latest typing rows:');
      print(describeRows(latestTyping));
      print('Chat app example finished.');
    } finally {
      await typingSub.cancel();
      await messagesSub.cancel();
      await conversationsSub.cancel();
    }
  } finally {
    await session.dispose();
  }
}
