import 'dart:async';

import 'package:drift/native.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kalam_sync/drift.dart' hide isNull, isNotNull;
import 'package:kalam_sync/kalam_sync.dart';

final class _Transport implements KalamSyncTransport {
  final changes = StreamController<KalamRemoteBatch>.broadcast();
  final connections = StreamController<KalamTransportConnection>.broadcast();
  @override
  Stream<KalamTransportConnection> get connectionStates => connections.stream;
  @override
  Stream<KalamRemoteBatch> subscribe({
    required String sql,
    required String subscriptionId,
    VersionId? from,
    int? batchSize,
    List<Object?>? params,
  }) => changes.stream;
  @override
  Future<void> ensureConnected() async {}
  @override
  Future<void> pause() async {}
  @override
  Future<void> resume() async {}
  @override
  Future<void> dispose() async {
    await changes.close();
    await connections.close();
  }
}

void main() {
  test(
    'legacy collision does not block durable consumer checkpoint and ack',
    () async {
      final db = KalamSyncDatabase(NativeDatabase.memory());
      final transport = _Transport();
      final store = KalamSyncStore(db);
      final registry = KalamActionRegistry.of([
        KalamActionDefinition<Map<String, Object?>>(
          key: 'refresh',
          codec: KalamActionCodec(encode: (v) => v, decode: (v) => v),
          execute: (_, _) async {},
        ),
      ]);
      final actions = KalamActionRunner(
        store: store,
        registry: registry,
        accountKey: 'account-a',
      );
      final coordinator = KalamSyncCoordinator(
        accountKey: 'account-a',
        store: store,
        transport: transport,
        actions: actions,
      );
      addTearDown(() async {
        await coordinator.dispose();
        await transport.dispose();
        await db.close();
      });
      await store.enqueue(
        const KalamActionDraft(
          id: 'mailbox:42',
          accountKey: 'account-a',
          actionKey: 'refresh',
          payloadJson: '{}',
        ),
      );
      final acknowledged = Completer<void>();
      await coordinator.subscribe(
        KalamEventConsumer.consume(
          id: 'mailbox',
          sql: 'SELECT * FROM public.user_updates',
          onEvent: (change, context) async {
            await context.enqueueForEvent(
              actionKey: 'refresh',
              consumerId: 'mailbox',
              rowKey: change.row['id'] as String,
              version: change.seq,
              payload: {'target': change.row['id']},
            );
          },
        ),
      );
      transport.changes.add(
        KalamRemoteBatch(
          changes: [
            const KalamRemoteChange(
              kind: KalamChangeKind.insert,
              seq: VersionId(42),
              row: {'id': 'row-1'},
            ),
          ],
          checkpoint: const VersionId(42),
          acknowledge: () async => acknowledged.complete(),
        ),
      );
      await acknowledged.future.timeout(const Duration(seconds: 3));
      expect(
        (await store.readCheckpoint(
          accountKey: 'account-a',
          subscriptionId: 'mailbox',
        ))?.seq,
        const VersionId(42),
      );
      expect((await db.select(db.kalamActions).get()).length, 2);
    },
  );

  test(
    'event actions scope identity and preserve legacy outbox records',
    () async {
      final db = KalamSyncDatabase(NativeDatabase.memory());
      addTearDown(db.close);
      final store = KalamSyncStore(db);
      final registry = KalamActionRegistry.of([
        KalamActionDefinition<Map<String, Object?>>(
          key: 'refresh',
          codec: KalamActionCodec(encode: (v) => v, decode: (v) => v),
          execute: (_, _) async {},
        ),
      ]);
      KalamConsumeContext context(String account) => KalamConsumeContext(
        actions: KalamActionRunner(
          store: store,
          registry: registry,
          accountKey: account,
        ),
      );
      Future<KalamActionRecord> enqueue(
        String account,
        String row, {
        String consumer = 'mailbox',
        String payload = 'same',
      }) => context(account).enqueueForEvent(
        actionKey: 'refresh',
        consumerId: consumer,
        rowKey: row,
        version: const VersionId(42),
        payload: {'value': payload},
      );
      await store.enqueue(
        const KalamActionDraft(
          id: 'mailbox:42',
          accountKey: 'a',
          actionKey: 'refresh',
          payloadJson: '{}',
        ),
      );
      final a = await enqueue('a', 'row-1');
      final repeat = await enqueue('a', 'row-1');
      final anotherRow = await enqueue('a', 'row-2');
      final anotherAccount = await enqueue('b', 'row-1');
      final anotherConsumer = await enqueue(
        'a',
        'row-1',
        consumer: 'different',
      );
      expect(repeat.id, a.id);
      expect(a.id, matches(RegExp(r'^kalam:event:v1:[0-9a-f]{64}$')));
      expect(
        {a.id, anotherRow.id, anotherAccount.id, anotherConsumer.id}.length,
        4,
      );
      expect(await store.readAction('mailbox:42'), isNotNull);
      await expectLater(
        enqueue('a', 'row-1', payload: 'different'),
        throwsStateError,
      );
      await expectLater(enqueue('a', ''), throwsArgumentError);

      final firstOrder = context('a').enqueueForEvent(
        actionKey: 'refresh',
        consumerId: 'mailbox',
        rowKey: 'row-ordered',
        version: const VersionId(43),
        payload: {
          'a': 1,
          'nested': {'x': 1, 'y': 2},
        },
      );
      final stored = await firstOrder;
      final reordered = await context('a').enqueueForEvent(
        actionKey: 'refresh',
        consumerId: 'mailbox',
        rowKey: 'row-ordered',
        version: const VersionId(43),
        payload: {
          'nested': {'y': 2, 'x': 1},
          'a': 1,
        },
      );
      expect(reordered.id, stored.id);
      await expectLater(
        context('a').enqueueForEvent(
          actionKey: 'refresh',
          consumerId: 'mailbox',
          rowKey: 'row-ordered',
          version: const VersionId(43),
          payload: {
            'nested': {'x': 1, 'y': 2},
            'a': 1.0,
          },
        ),
        throwsStateError,
      );
    },
  );
}
