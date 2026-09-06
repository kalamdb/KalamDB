import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:kalam_sync/kalam_sync.dart';
import 'package:kalam_sync_generator_example/chat_actions.dart';
import 'package:kalam_sync_generator_example/chat_http_api.dart';
import 'package:kalam_sync_generator_example/chat_models.dart';
import 'package:kalam_sync_generator_example/chat_tables.dart';

String _env(String key, String fallback) {
  final value = Platform.environment[key];
  if (value == null || value.trim().isEmpty) return fallback;
  return value;
}

final kalamUrl = _env('KALAMDB_URL', _env('KALAM_URL', 'http://localhost:2900'));
final chatApiUrl = _env('KALAM_CHAT_URL', 'http://127.0.0.1:8787');
final username = _env('KALAMDB_USER', _env('KALAM_USER', 'root'));
final password = _env(
  'KALAMDB_PASSWORD',
  _env('KALAM_PASS', 'kalamdb123'),
);
const namespace = 'chat';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  final api = ChatHttpApi(baseUrl: Uri.parse(chatApiUrl));
  final kalam = await Kalam.open(
    url: kalamUrl,
    subject: username,
    namespace: namespace,
    authProvider: () async => Auth.basic(username, password),
    actionDefinitions: chatActionsDefinitions(ChatActions(api)),
  );
  runApp(
    KalamScope(
      kalam: kalam,
      child: ChatApp(kalam: kalam, author: username),
    ),
  );
}

final class ChatApp extends StatefulWidget {
  const ChatApp({required this.kalam, required this.author, super.key});

  final Kalam kalam;
  final String author;

  @override
  State<ChatApp> createState() => _ChatAppState();
}

final class _ChatAppState extends State<ChatApp> {
  late final conversations = widget.kalam.table(
    chatConversationsSpec('$namespace.conversations'),
  );
  late final messages = widget.kalam.table(
    chatMessagesSpec('$namespace.messages'),
  );
  late final actions = ChatActionsQueue(widget.kalam.actions);
  final subscriptions = <KalamSyncSubscription>[];
  String? openConversationId;
  var offline = false;

  @override
  void initState() {
    super.initState();
    _start();
  }

  Future<void> _start() async {
    subscriptions.add(
      await widget.kalam.subscribe(
        conversations.consumer(sql: 'SELECT * FROM $namespace.conversations'),
      ),
    );
    subscriptions.add(
      await widget.kalam.subscribe(
        messages.consumer(sql: 'SELECT * FROM $namespace.messages'),
      ),
    );
  }

  @override
  void dispose() {
    for (final subscription in subscriptions) {
      unawaited(subscription.cancel());
    }
    super.dispose();
  }

  Future<void> _toggleOffline() async {
    if (offline) {
      await widget.kalam.resume();
    } else {
      await widget.kalam.pause();
    }
    setState(() => offline = !offline);
  }

  Future<void> _createConversation() async {
    final now = DateTime.now().toUtc();
    final conversation = Conversation(
      id: Kalam.id(),
      title: 'Chat ${now.toIso8601String().substring(11, 19)}',
      createdAt: now,
      updatedAt: now,
    );
    await conversations.insert(conversation, actionId: Kalam.id());
    setState(() => openConversationId = conversation.id);
  }

  Future<void> _send(String conversationId, String text) async {
    final now = DateTime.now().toUtc();
    final message = ChatMessage(
      id: Kalam.id(),
      conversationId: conversationId,
      role: ChatMessageRole.user,
      author: widget.author,
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
        author: widget.author,
      ),
      orderingKey: conversationId,
      optimistic: messages.optimisticInsert(message),
    );
  }

  Future<void> _markRead(ChatMessage message) async {
    if (message.role != ChatMessageRole.assistant) return;
    final readAt = DateTime.now().toUtc();
    await actions.markMsgRead(
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

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Kalam chat',
      home: Scaffold(
        appBar: AppBar(
          title: Text(offline ? 'Kalam chat (offline)' : 'Kalam chat'),
          actions: [
            IconButton(
              tooltip: offline ? 'Go online' : 'Go offline',
              onPressed: _toggleOffline,
              icon: Icon(offline ? Icons.cloud_off : Icons.cloud_queue),
            ),
          ],
        ),
        body: openConversationId == null
            ? _ConversationList(
                conversations: conversations,
                onOpen: (id) => setState(() => openConversationId = id),
              )
            : _ConversationView(
                conversationId: openConversationId!,
                messages: messages,
                onBack: () => setState(() => openConversationId = null),
                onSend: (text) => _send(openConversationId!, text),
                onMarkRead: _markRead,
              ),
        floatingActionButton: openConversationId == null
            ? FloatingActionButton(
                onPressed: _createConversation,
                child: const Icon(Icons.add_comment),
              )
            : null,
      ),
    );
  }
}

final class _ConversationList extends StatelessWidget {
  const _ConversationList({
    required this.conversations,
    required this.onOpen,
  });

  final KalamTableBinding<Conversation> conversations;
  final ValueChanged<String> onOpen;

  @override
  Widget build(BuildContext context) {
    return StreamBuilder<List<KalamSyncedRow<Conversation>>>(
      stream: conversations.watchWithSyncState(),
      builder: (context, snapshot) {
        final rows = [...snapshot.data ?? const <KalamSyncedRow<Conversation>>[]]
          ..sort((a, b) => b.value.updatedAt.compareTo(a.value.updatedAt));
        if (rows.isEmpty) {
          return const Center(child: Text('Create a conversation to start.'));
        }
        return ListView(
          children: [
            for (final row in rows)
              ListTile(
                title: Text(row.value.title),
                subtitle: Text(row.isSynced ? 'Synced' : 'Pending sync'),
                trailing: Icon(
                  row.isSynced ? Icons.cloud_done : Icons.cloud_upload,
                ),
                onTap: () => onOpen(row.value.id),
              ),
          ],
        );
      },
    );
  }
}

final class _ConversationView extends StatefulWidget {
  const _ConversationView({
    required this.conversationId,
    required this.messages,
    required this.onBack,
    required this.onSend,
    required this.onMarkRead,
  });

  final String conversationId;
  final KalamTableBinding<ChatMessage> messages;
  final VoidCallback onBack;
  final Future<void> Function(String text) onSend;
  final Future<void> Function(ChatMessage message) onMarkRead;

  @override
  State<_ConversationView> createState() => _ConversationViewState();
}

final class _ConversationViewState extends State<_ConversationView> {
  final controller = TextEditingController();

  @override
  void dispose() {
    controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Column(
      children: [
        ListTile(
          leading: IconButton(
            onPressed: widget.onBack,
            icon: const Icon(Icons.arrow_back),
          ),
          title: const Text('Messages'),
          subtitle: const Text('Tap an assistant bubble to mark it read'),
        ),
        Expanded(
          child: StreamBuilder<List<KalamSyncedRow<ChatMessage>>>(
            stream: widget.messages.watchWithSyncState(),
            builder: (context, snapshot) {
              final rows =
                  [
                    ...snapshot.data ?? const <KalamSyncedRow<ChatMessage>>[],
                  ].where((row) => row.value.conversationId == widget.conversationId).toList()
                    ..sort(
                      (a, b) => a.value.createdAt.compareTo(b.value.createdAt),
                    );
              return ListView(
                padding: const EdgeInsets.all(12),
                children: [
                  for (final row in rows)
                    Align(
                      alignment: row.value.isAssistant
                          ? Alignment.centerLeft
                          : Alignment.centerRight,
                      child: GestureDetector(
                        onTap: () => widget.onMarkRead(row.value),
                        child: Card(
                          color: row.value.isAssistant
                              ? Colors.grey.shade200
                              : Colors.blue.shade50,
                          child: Padding(
                            padding: const EdgeInsets.all(12),
                            child: Column(
                              crossAxisAlignment: CrossAxisAlignment.start,
                              children: [
                                Text(row.value.text),
                                const SizedBox(height: 4),
                                Text(
                                  '${row.value.status.name} · ${row.sync.phase.name}',
                                  style: Theme.of(context).textTheme.bodySmall,
                                ),
                              ],
                            ),
                          ),
                        ),
                      ),
                    ),
                ],
              );
            },
          ),
        ),
        Padding(
          padding: const EdgeInsets.all(12),
          child: Row(
            children: [
              Expanded(
                child: TextField(
                  controller: controller,
                  decoration: const InputDecoration(
                    hintText: 'Message',
                    border: OutlineInputBorder(),
                  ),
                  onSubmitted: (value) async {
                    final text = value.trim();
                    if (text.isEmpty) return;
                    controller.clear();
                    await widget.onSend(text);
                  },
                ),
              ),
              IconButton(
                onPressed: () async {
                  final text = controller.text.trim();
                  if (text.isEmpty) return;
                  controller.clear();
                  await widget.onSend(text);
                },
                icon: const Icon(Icons.send),
              ),
            ],
          ),
        ),
      ],
    );
  }
}
