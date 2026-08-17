import 'dart:async';
import 'dart:io';

import 'package:drift/native.dart';
import 'package:kalam_sync/drift.dart' hide isNotNull, isNull;
import 'package:kalam_sync/kalam_sync.dart';
import 'package:kalam_sync_chat_backend/chat_backend.dart';
import 'package:kalam_sync_generator_example/chat_http_api.dart';
import 'package:kalam_sync_generator_example/chat_actions.dart';
import 'package:kalam_sync_generator_example/chat_models.dart';
import 'package:kalam_sync_generator_example/chat_tables.dart';

import '../../../../link/test/e2e/helpers.dart';

final class MemoryDatabaseFactory implements KalamDatabaseFactory {
  MemoryDatabaseFactory() {
    driftRuntimeOptions.dontWarnAboutMultipleDatabases = true;
  }

  @override
  Future<KalamSyncDatabase> open(KalamAccountIdentity identity) async {
    return KalamSyncDatabase(NativeDatabase.memory());
  }
}

final class CountingTransport implements KalamSyncTransport {
  CountingTransport(this.delegate);

  final KalamLinkTransport delegate;
  final Map<String, List<int>> deliveredBatchSizes = {};
  final Map<String, List<SeqId>> acknowledged = {};
  final Map<String, List<int>> deliveredSeqs = {};

  @override
  Stream<KalamTransportConnection> get connectionStates =>
      delegate.connectionStates;

  @override
  Stream<KalamRemoteBatch> subscribe({
    required String sql,
    required String subscriptionId,
    SeqId? from,
    int? batchSize,
  }) {
    return delegate
        .subscribe(
          sql: sql,
          subscriptionId: subscriptionId,
          from: from,
          batchSize: batchSize,
        )
        .map((batch) {
          if (batch.checkpoint != null) {
            deliveredBatchSizes
                .putIfAbsent(subscriptionId, () => [])
                .add(batch.changes.length);
            deliveredSeqs.putIfAbsent(subscriptionId, () => []).addAll(
              batch.changes.map((change) => change.seq.value),
            );
          }
          return KalamRemoteBatch(
            changes: batch.changes,
            checkpoint: batch.checkpoint,
            acknowledge: () async {
              await batch.acknowledge();
              final checkpoint = batch.checkpoint;
              if (checkpoint != null) {
                acknowledged
                    .putIfAbsent(subscriptionId, () => [])
                    .add(checkpoint);
              }
            },
          );
        });
  }

  @override
  Future<void> ensureConnected() => delegate.ensureConnected();

  @override
  Future<void> pause() => delegate.pause();

  @override
  Future<void> resume() => delegate.resume();

  @override
  Future<void> dispose() => delegate.dispose();
}

final class ChatE2eSession {
  ChatE2eSession({
    required this.namespace,
    required this.kalam,
    required this.backend,
    required this.api,
    required this.conversations,
    required this.messages,
    required this.actions,
    required this.transport,
  });

  final String namespace;
  final Kalam kalam;
  final ChatBackend backend;
  final ChatHttpApi api;
  final KalamTableBinding<Conversation> conversations;
  final KalamTableBinding<ChatMessage> messages;
  final ChatActionsQueue actions;
  final CountingTransport? transport;

  String get conversationsTable => '$namespace.conversations';
  String get messagesTable => '$namespace.messages';

  Future<void> subscribe({int? messageBatchSize}) async {
    await kalam.subscribe(
      conversations.consumer(sql: 'SELECT * FROM $conversationsTable'),
    );
    await kalam.subscribe(
      messages.consumer(
        sql: 'SELECT * FROM $messagesTable',
        batchSize: messageBatchSize,
      ),
    );
    await kalam.transport.ensureConnected();
  }

  Future<Conversation> createConversation(String title) async {
    final now = DateTime.now().toUtc();
    final conversation = Conversation(
      id: Kalam.id(),
      title: title,
      createdAt: now,
      updatedAt: now,
    );
    await conversations.insert(conversation, actionId: Kalam.id());
    await waitForRow<Conversation>(
      conversations,
      (row) => row.value.id == conversation.id && row.isSynced,
    );
    return conversation;
  }

  Future<ChatMessage> sendMessage({
    required String conversationId,
    required String text,
    String? messageId,
    String? actionId,
  }) async {
    final now = DateTime.now().toUtc();
    final message = ChatMessage(
      id: messageId ?? Kalam.id(),
      conversationId: conversationId,
      role: ChatMessageRole.user,
      author: adminUser,
      text: text,
      status: ChatDeliveryStatus.pending,
      createdAt: now,
    );
    await actions.sendMessage(
      SendMessageArgs(
        messageId: message.id,
        conversationId: conversationId,
        text: text,
        createdAt: now,
        author: adminUser,
      ),
      actionId: actionId,
      orderingKey: conversationId,
      optimistic: messages.optimisticInsert(message),
    );
    return message;
  }

  Future<void> markRead(ChatMessage message) {
    final readAt = DateTime.now().toUtc();
    return actions.markMsgRead(
      MarkMessageReadArgs(
        messageId: message.id,
        conversationId: message.conversationId,
        readAt: readAt,
      ),
      orderingKey: message.conversationId,
      optimistic: messages.optimisticInsert(
        message.copyWith(status: ChatDeliveryStatus.read, readAt: readAt),
      ),
    );
  }

  Future<void> close() async {
    await kalam.dispose();
    api.close();
    try {
      await backend.executeSql('DROP TABLE IF EXISTS $messagesTable');
      await backend.executeSql('DROP TABLE IF EXISTS $conversationsTable');
    } catch (_) {}
    await backend.close();
  }
}

Future<ChatE2eSession> openChatSession({
  bool countTransport = false,
}) async {
  final namespace = uniqueName('chat_e2e');
  final restPort = await _freePort();
  final backend = ChatBackend(
    kalamUrl: serverUrl,
    namespace: namespace,
    username: adminUser,
    password: adminPass,
    port: restPort,
  );
  await backend.start();
  final api = ChatHttpApi(baseUrl: backend.url);
  CountingTransport? counted;
  Future<Auth> authProvider() async => Auth.basic(adminUser, adminPass);
  final actionDefinitions = chatActionsDefinitions(ChatActions(api));
  final Kalam kalam;
  if (countTransport) {
    final link = await KalamLinkTransport.connect(
      url: serverUrl,
      authProvider: authProvider,
    );
    counted = CountingTransport(link);
    final identity = KalamAccountIdentity(
      serverUrl: serverUrl,
      subject: adminUser,
      namespace: namespace,
    );
    kalam = Kalam.fromComponents(
      identity: identity,
      database: await MemoryDatabaseFactory().open(identity),
      transport: counted,
      dmlClient: link.client,
      actionDefinitions: actionDefinitions,
    );
  } else {
    kalam = await Kalam.open(
      url: serverUrl,
      subject: adminUser,
      namespace: namespace,
      authProvider: authProvider,
      databaseFactory: MemoryDatabaseFactory(),
      actionDefinitions: actionDefinitions,
    );
  }
  return ChatE2eSession(
    namespace: namespace,
    kalam: kalam,
    backend: backend,
    api: api,
    conversations: kalam.table(
      chatConversationsSpec('$namespace.conversations'),
    ),
    messages: kalam.table(chatMessagesSpec('$namespace.messages')),
    actions: ChatActionsQueue(kalam.actions),
    transport: counted,
  );
}

Future<Kalam> openPeer(ChatE2eSession session) {
  return Kalam.open(
    url: serverUrl,
    subject: adminUser,
    namespace: session.namespace,
    authProvider: () async => Auth.basic(adminUser, adminPass),
    databaseFactory: MemoryDatabaseFactory(),
    actionDefinitions: chatActionsDefinitions(ChatActions(session.api)),
  );
}

Future<KalamSyncedRow<T>> waitForRow<T>(
  KalamTableBinding<T> binding,
  bool Function(KalamSyncedRow<T> row) test, {
  Duration timeout = const Duration(seconds: 20),
}) async {
  final rows = await binding
      .watchWithSyncState()
      .firstWhere((rows) => rows.any(test))
      .timeout(timeout);
  return rows.firstWhere(test);
}

Future<List<KalamSyncedRow<ChatMessage>>> waitForMessages(
  KalamTableBinding<ChatMessage> binding,
  bool Function(List<KalamSyncedRow<ChatMessage>> rows) test, {
  Duration timeout = const Duration(seconds: 20),
}) {
  return binding.watchWithSyncState().firstWhere(test).timeout(timeout);
}

Future<KalamActionRecord> waitForAction(Kalam kalam, String actionId) async {
  final deadline = DateTime.now().add(const Duration(seconds: 15));
  while (DateTime.now().isBefore(deadline)) {
    final action = await kalam.store.readAction(actionId);
    if (action != null && action.isTerminal) return action;
    await Future<void>.delayed(const Duration(milliseconds: 20));
  }
  throw TimeoutException('Action $actionId did not finish.');
}

Future<KalamCheckpoint> waitForCheckpoint(
  Kalam kalam,
  String subscriptionId,
) async {
  final deadline = DateTime.now().add(const Duration(seconds: 15));
  while (DateTime.now().isBefore(deadline)) {
    final checkpoint = await kalam.store.readCheckpoint(
      accountKey: kalam.identity.accountKey,
      subscriptionId: subscriptionId,
    );
    if (checkpoint != null) return checkpoint;
    await Future<void>.delayed(const Duration(milliseconds: 20));
  }
  throw TimeoutException('No checkpoint committed for $subscriptionId.');
}

Future<int> _freePort() async {
  final socket = await ServerSocket.bind(InternetAddress.loopbackIPv4, 0);
  final port = socket.port;
  await socket.close();
  return port;
}
