import 'package:flutter_test/flutter_test.dart';
import 'package:kalam_sync/kalam_sync.dart';
import 'package:kalam_sync_generator_example/chat_models.dart';

import '../../../link/test/e2e/helpers.dart';
import 'support/chat_e2e_harness.dart';

void main() {
  setUpAll(ensureSdkReady);

  test(
    'offlineEnqueueThenReconnect',
    () async {
      final session = await openChatSession();
      addTearDown(session.close);
      await session.subscribe();

      final conversation = await session.createConversation('Offline inbox');
      expect(conversation.title, 'Offline inbox');

      await session.kalam.pause();
      final pending = session.sendMessage(
        conversationId: conversation.id,
        text: 'hello offline',
      );
      final queued = await pending;
      final optimistic = await waitForRow<ChatMessage>(
        session.messages,
        (row) =>
            row.value.id == queued.id &&
            row.value.text == 'hello offline' &&
            row.sync.isPending,
      );
      expect(optimistic.sync.phase, KalamRowSyncPhase.pending);

      await session.kalam.resume();
      final synced = await waitForRow<ChatMessage>(
        session.messages,
        (row) => row.value.id == queued.id && row.sync.isSynced,
      );
      expect(synced.value.status, isNot(ChatDeliveryStatus.pending));
      expect(synced.sync.phase, KalamRowSyncPhase.synced);

      final assistant = await waitForRow<ChatMessage>(
        session.messages,
        (row) =>
            row.value.id == 'ai-${queued.id}' &&
            row.value.role == ChatMessageRole.assistant &&
            row.sync.isSynced,
      );
      expect(assistant.value.conversationId, conversation.id);
      expect(assistant.value.author, 'demo-bot');
    },
    skip: skipIfNoIntegration,
    timeout: const Timeout(Duration(seconds: 60)),
  );

  test(
    'replicaOnlyRejectsDirectMutations',
    () async {
      final session = await openChatSession();
      addTearDown(session.close);
      await session.subscribe();
      final conversation = await session.createConversation('Authoritative');

      expect(
        () => session.messages.insert(
          ChatMessage(
            id: Kalam.id(),
            conversationId: conversation.id,
            role: ChatMessageRole.user,
            author: adminUser,
            text: 'nope',
            status: ChatDeliveryStatus.sent,
            createdAt: DateTime.now().toUtc(),
          ),
          actionId: Kalam.id(),
        ),
        throwsA(
          isA<StateError>().having(
            (error) => error.toString(),
            'message',
            contains('replicaOnly'),
          ),
        ),
      );

      final sent = await session.sendMessage(
        conversationId: conversation.id,
        text: 'via generated action',
      );
      final user = await waitForRow<ChatMessage>(
        session.messages,
        (row) =>
            row.value.id == sent.id &&
            row.value.status == ChatDeliveryStatus.delivered &&
            row.sync.isSynced,
      );
      expect(user.value.status, ChatDeliveryStatus.delivered);

      final assistant = await waitForRow<ChatMessage>(
        session.messages,
        (row) => row.value.id == 'ai-${sent.id}' && row.sync.isSynced,
      );
      await session.markRead(assistant.value);
      final read = await waitForRow<ChatMessage>(
        session.messages,
        (row) =>
            row.value.id == assistant.value.id &&
            row.value.status == ChatDeliveryStatus.read &&
            row.sync.isSynced,
      );
      expect(read.value.readAt, isNotNull);
    },
    skip: skipIfNoIntegration,
    timeout: const Timeout(Duration(seconds: 60)),
  );

  test(
    'optimisticUiThenServerEcho',
    () async {
      final session = await openChatSession();
      addTearDown(session.close);
      await session.subscribe();
      final conversation = await session.createConversation('Optimistic');
      final phases = <KalamRowSyncPhase>{};
      final sub = session.messages.watchWithSyncState().listen((rows) {
        for (final row in rows) {
          if (row.value.text == 'echo me') {
            phases.add(row.sync.phase);
          }
        }
      });
      addTearDown(sub.cancel);

      await session.kalam.pause();
      final actionId = 'echo-${DateTime.now().microsecondsSinceEpoch}';
      final queued = await session.sendMessage(
        conversationId: conversation.id,
        text: 'echo me',
        actionId: actionId,
      );
      await waitForRow<ChatMessage>(
        session.messages,
        (row) => row.value.id == queued.id && row.sync.isPending,
      );

      await session.kalam.resume();
      final completed = await waitForAction(session.kalam, actionId);
      expect(completed.status, KalamActionStatus.succeeded);
      final afterRest = await waitForRow<ChatMessage>(
        session.messages,
        (row) =>
            row.value.id == queued.id &&
            (row.sync.phase == KalamRowSyncPhase.awaitingServerEcho ||
                row.sync.isSynced),
      );
      if (!afterRest.sync.isSynced) {
        expect(afterRest.sync.phase, KalamRowSyncPhase.awaitingServerEcho);
      }
      final synced = await waitForRow<ChatMessage>(
        session.messages,
        (row) => row.value.id == queued.id && row.sync.isSynced,
      );
      expect(synced.value.text, 'echo me');
      expect(phases, contains(KalamRowSyncPhase.pending));
      expect(phases, contains(KalamRowSyncPhase.synced));
    },
    skip: skipIfNoIntegration,
    timeout: const Timeout(Duration(seconds: 60)),
  );

  test(
    'readReceiptsOnlineAndOffline',
    () async {
      final session = await openChatSession();
      addTearDown(session.close);
      await session.subscribe();
      final conversation = await session.createConversation('Receipts');

      final first = await session.sendMessage(
        conversationId: conversation.id,
        text: 'please read this',
      );
      final firstAssistant = await waitForRow<ChatMessage>(
        session.messages,
        (row) => row.value.id == 'ai-${first.id}' && row.sync.isSynced,
      );
      await session.markRead(firstAssistant.value);
      await waitForRow<ChatMessage>(
        session.messages,
        (row) =>
            row.value.id == firstAssistant.value.id &&
            row.value.status == ChatDeliveryStatus.read,
      );

      final peer = await openPeer(session);
      addTearDown(peer.dispose);
      final peerMessages = peer.table(
        session.messages.spec,
      );
      await peer.subscribe(
        peerMessages.consumer(sql: 'SELECT * FROM ${session.messagesTable}'),
      );
      await waitForRow<ChatMessage>(
        peerMessages,
        (row) =>
            row.value.id == firstAssistant.value.id &&
            row.value.status == ChatDeliveryStatus.read,
      );

      await session.kalam.pause();
      final second = await session.sendMessage(
        conversationId: conversation.id,
        text: 'offline then read',
      );
      await session.kalam.resume();
      final secondAssistant = await waitForRow<ChatMessage>(
        session.messages,
        (row) => row.value.id == 'ai-${second.id}' && row.sync.isSynced,
      );

      await session.kalam.pause();
      await session.markRead(secondAssistant.value);
      await session.kalam.resume();
      await waitForRow<ChatMessage>(
        session.messages,
        (row) =>
            row.value.id == secondAssistant.value.id &&
            row.value.status == ChatDeliveryStatus.read &&
            row.sync.isSynced,
      );
      await waitForRow<ChatMessage>(
        peerMessages,
        (row) =>
            row.value.id == secondAssistant.value.id &&
            row.value.status == ChatDeliveryStatus.read,
      );

      await session.markRead(secondAssistant.value);
      await waitForRow<ChatMessage>(
        session.messages,
        (row) =>
            row.value.id == secondAssistant.value.id &&
            row.value.status == ChatDeliveryStatus.read,
      );
    },
    skip: skipIfNoIntegration,
    timeout: const Timeout(Duration(seconds: 90)),
  );

  test(
    'catchUpAfterGapAndBackpressure',
    () async {
      final session = await openChatSession(countTransport: true);
      addTearDown(session.close);
      await session.subscribe(messageBatchSize: 10);
      final conversation = await session.createConversation('Catch-up');
      final transport = session.transport!;
      final subscriptionId = session.messages.spec.subscriptionId;

      final seed = await session.sendMessage(
        conversationId: conversation.id,
        text: 'seed checkpoint',
      );
      await waitForRow<ChatMessage>(
        session.messages,
        (row) => row.value.id == seed.id && row.sync.isSynced,
      );
      await waitForCheckpoint(session.kalam, subscriptionId);
      await session.kalam.pause().timeout(const Duration(seconds: 10));

      const burst = 40;
      for (var index = 0; index < burst; index++) {
        await session.backend.executeSql(
          'INSERT INTO ${session.messagesTable} ('
          'id, conversation_id, role, author, text, status, created_at'
          ') VALUES ('
          "'burst-$index', "
          "${_sql(conversation.id)}, "
          "'assistant', "
          "'demo-bot', "
          "'burst $index', "
          "'delivered', "
          "${_sql(DateTime.utc(2026, 8, 17, 0, 0, index).toIso8601String())}"
          ')',
        );
      }

      await session.kalam.resume().timeout(const Duration(seconds: 15));
      final rows = await waitForMessages(
        session.messages,
        (rows) =>
            rows
                .where((row) => row.value.id.startsWith('burst-'))
                .length ==
            burst,
        timeout: const Duration(seconds: 45),
      );
      final burstRows =
          rows.where((row) => row.value.id.startsWith('burst-')).toList()
            ..sort((a, b) => a.value.createdAt.compareTo(b.value.createdAt));
      expect(burstRows, hasLength(burst));
      expect(
        burstRows.map((row) => row.value.id).toSet(),
        hasLength(burst),
      );
      expect(
        burstRows.first.value.id,
        'burst-0',
      );
      expect(burstRows.last.value.id, 'burst-39');
      expect(burstRows.every((row) => row.sync.isSynced), isTrue);

      final seqs = transport.deliveredSeqs[subscriptionId] ?? const <int>[];
      expect(seqs, isNotEmpty);
      expect(seqs.toSet(), hasLength(seqs.length), reason: 'duplicate seqs');
      var previous = seqs.first;
      for (final seq in seqs.skip(1)) {
        expect(seq, greaterThan(previous));
        previous = seq;
      }

      final sizes = transport.deliveredBatchSizes[subscriptionId] ?? const [];
      await waitForCondition(
        () =>
            (transport.acknowledged[subscriptionId]?.length ?? 0) >=
            sizes.where((size) => size > 0).length,
        timeout: const Duration(seconds: 10),
        poll: const Duration(milliseconds: 20),
      );
      expect(transport.acknowledged[subscriptionId], isNotEmpty);
      final checkpoint = await waitForCheckpoint(
        session.kalam,
        subscriptionId,
      );
      expect(
        checkpoint.seq,
        transport.acknowledged[subscriptionId]!.last,
      );
    },
    skip: skipIfNoIntegration,
    timeout: const Timeout(Duration(seconds: 90)),
  );
}

String _sql(String value) => "'${value.replaceAll("'", "''")}'";
