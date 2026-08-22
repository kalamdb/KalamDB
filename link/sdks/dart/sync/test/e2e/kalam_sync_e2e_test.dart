import 'dart:async';

import 'package:drift/native.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kalam_sync/drift.dart' hide isNotNull, isNull;
import 'package:kalam_sync/kalam_sync.dart';

import '../../../link/test/e2e/helpers.dart';

final class E2eMessage {
  const E2eMessage({required this.id, required this.text});

  final String id;
  final String text;
}

final class MemoryDatabaseFactory implements KalamDatabaseFactory {
  @override
  Future<KalamSyncDatabase> open(KalamAccountIdentity identity) async {
    return KalamSyncDatabase(NativeDatabase.memory());
  }
}

void main() {
  test(
    'queues offline, executes the action, applies the echo, and checkpoints',
    () async {
      await ensureSdkReady();
      final setup = await connectJwtClient();
      final namespace = uniqueName('sync_e2e');
      final table = '$namespace.messages';
      await setup.query('CREATE NAMESPACE IF NOT EXISTS $namespace');
      final created = await setup.query(
        'CREATE TABLE IF NOT EXISTS $table ('
        'id TEXT PRIMARY KEY, text TEXT NOT NULL)',
      );
      expect(created.success, isTrue, reason: created.error?.toString());
      addTearDown(() async {
        await setup.query('DROP TABLE IF EXISTS $table');
        await setup.dispose();
      });

      final localRows = <E2eMessage>[];
      final localChanges = StreamController<List<E2eMessage>>.broadcast();
      addTearDown(localChanges.close);

      final send = KalamActionDefinition<Map<String, Object?>>(
        key: 'messages.send',
        codec: KalamActionCodec(
          encode: (payload) => payload,
          decode: (json) => json,
        ),
        execute: (context, payload) async {
          final response = await setup.query(
            'INSERT INTO $table (id, text) VALUES (\$1, \$2)',
            params: [payload['id'], payload['text']],
          );
          if (!response.success) {
            throw KalamPermanentActionException(response.error.toString());
          }
        },
      );
      final kalam = await Kalam.open(
        url: serverUrl,
        subject: adminUser,
        namespace: namespace,
        authProvider: () async => Auth.basic(adminUser, adminPass),
        databaseFactory: MemoryDatabaseFactory(),
        actionDefinitions: [send],
      );
      addTearDown(kalam.dispose);

      final wakeAction = await kalam.actions.enqueue(
        actionKey: 'messages.send',
        actionId: 'wake-${DateTime.now().microsecondsSinceEpoch}',
        payload: const {'id': 'message-wake', 'text': 'no active consumer'},
      );
      expect(
        (await _waitForAction(kalam, wakeAction.id)).status,
        KalamActionStatus.succeeded,
      );
      await _waitForServerRow(setup, table, 'message-wake');

      final binding = KalamTableBinding<E2eMessage>(
        spec: KalamTableSpec(
          tableId: table,
          keyColumn: 'id',
          subscriptionId: '$table/all',
          mode: KalamSyncMode.replicaOnly,
          keyOf: (row) => row.id,
          encode: (row) => {'id': row.id, 'text': row.text},
          decode: (json) => E2eMessage(
            id: json['id']! as String,
            text: json['text']! as String,
          ),
        ),
        accountKey: kalam.identity.accountKey,
        store: kalam.store,
        watchLocal: () async* {
          yield List.of(localRows);
          yield* localChanges.stream;
        },
        upsertLocal: (message) async {
          localRows.removeWhere((row) => row.id == message.id);
          localRows.add(message);
          localChanges.add(List.of(localRows));
        },
        deleteLocal: (id) async {
          localRows.removeWhere((row) => row.id == id);
          localChanges.add(List.of(localRows));
        },
      );
      final subscription = await kalam.subscribe(
        binding.consumer(sql: 'SELECT * FROM $table'),
      );
      addTearDown(subscription.cancel);

      await kalam.pause();
      final pending = binding.watchWithSyncState().firstWhere(
        (rows) => rows.any(
          (row) => row.value.id == 'message-1' && row.sync.isPending,
        ),
      );
      final queued = await kalam.actions.enqueue(
        actionKey: 'messages.send',
        actionId: 'action-${DateTime.now().microsecondsSinceEpoch}',
        payload: const {'id': 'message-1', 'text': 'hello offline'},
        optimisticRow: KalamOptimisticRow(
          tableId: table,
          rowKey: 'message-1',
          phase: KalamRowSyncPhase.pending,
          pendingValuesJson: '{"id":"message-1","text":"hello offline"}',
        ),
        applyOptimistic: () async {
          localRows.add(
            const E2eMessage(id: 'message-1', text: 'hello offline'),
          );
          localChanges.add(List.of(localRows));
        },
      );
      expect(queued.status, KalamActionStatus.queued);
      await pending.timeout(const Duration(seconds: 5));

      final synced = binding.watchWithSyncState().firstWhere(
        (rows) =>
            rows.any((row) => row.value.id == 'message-1' && row.sync.isSynced),
      );
      await kalam.resume();
      final rows = await synced.timeout(const Duration(seconds: 15));
      final checkpoint = await _waitForCheckpoint(kalam, '$table/all');

      final message = rows.singleWhere((row) => row.value.id == 'message-1');
      expect(message.value.text, 'hello offline');
      expect(message.sync.phase, KalamRowSyncPhase.synced);
      expect(checkpoint, isNotNull);

      final direct = kalam.table(
        KalamTableSpec<E2eMessage>(
          tableId: table,
          keyColumn: 'id',
          mode: KalamSyncMode.bidirectional,
          keyOf: (row) => row.id,
          encode: (row) => {'id': row.id, 'text': row.text},
          decode: (json) => E2eMessage(
            id: json['id']! as String,
            text: json['text']! as String,
          ),
        ),
      );
      final directSynced = direct.watchWithSyncState().firstWhere(
        (rows) =>
            rows.any((row) => row.value.id == 'message-2' && row.sync.isSynced),
      );
      await direct.insert(
        const E2eMessage(id: 'message-2', text: 'direct DML'),
        actionId: Kalam.id(),
      );
      final directRows = await directSynced.timeout(
        const Duration(seconds: 15),
      );
      expect(directRows.any((row) => row.value.id == 'message-2'), isTrue);

      final deleteAction = await direct.delete(
        'message-2',
        actionId: Kalam.id(),
      );
      final deleted = await _waitForAction(kalam, deleteAction.id);
      expect(deleted.status, KalamActionStatus.succeeded);
      await _waitForServerDelete(setup, table, 'message-2');
    },
    skip: skipIfNoIntegration,
    timeout: const Timeout(Duration(seconds: 45)),
  );
}

Future<KalamActionRecord> _waitForAction(Kalam kalam, String actionId) async {
  final deadline = DateTime.now().add(const Duration(seconds: 10));
  while (DateTime.now().isBefore(deadline)) {
    final action = await kalam.store.readAction(actionId);
    if (action != null && action.isTerminal) return action;
    await Future<void>.delayed(const Duration(milliseconds: 20));
  }
  throw TimeoutException('Action $actionId did not finish.');
}

Future<void> _waitForServerDelete(
  KalamClient client,
  String table,
  String rowId,
) async {
  final deadline = DateTime.now().add(const Duration(seconds: 10));
  while (DateTime.now().isBefore(deadline)) {
    final remaining = await client.query(
      'SELECT id FROM $table WHERE id = \$1',
      params: [rowId],
    );
    if (remaining.rows.isEmpty) return;
    await Future<void>.delayed(const Duration(milliseconds: 20));
  }
  throw TimeoutException('Backend row $rowId was not deleted from $table.');
}

Future<void> _waitForServerRow(
  KalamClient client,
  String table,
  String rowId,
) async {
  final deadline = DateTime.now().add(const Duration(seconds: 10));
  while (DateTime.now().isBefore(deadline)) {
    final rows = await client.query(
      'SELECT id FROM $table WHERE id = \$1',
      params: [rowId],
    );
    if (rows.rows.isNotEmpty) return;
    await Future<void>.delayed(const Duration(milliseconds: 20));
  }
  throw TimeoutException('Backend row $rowId was not inserted into $table.');
}

Future<KalamCheckpoint> _waitForCheckpoint(
  Kalam kalam,
  String subscriptionId,
) async {
  final deadline = DateTime.now().add(const Duration(seconds: 5));
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
