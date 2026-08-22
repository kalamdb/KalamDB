import 'package:drift/native.dart';
import 'package:kalam_sync/drift.dart' hide isNotNull, isNull;
import 'package:flutter_test/flutter_test.dart';
import 'package:kalam_sync/kalam_sync.dart';

void main() {
  late KalamSyncDatabase database;
  late KalamSyncStore store;
  final now = DateTime.utc(2026, 8, 16, 12);

  setUp(() {
    database = KalamSyncDatabase(NativeDatabase.memory());
    store = KalamSyncStore(database, clock: () => now);
  });

  tearDown(() => database.close());

  test('atomically enqueues an action and optimistic row state', () async {
    final action = await store.enqueue(
      const KalamActionDraft(
        id: 'action-1',
        accountKey: 'server/user-a',
        actionKey: 'messages.send',
        version: 1,
        payloadJson: '{"text":"hello"}',
        orderingKey: 'conversation-1',
      ),
      optimisticRow: const KalamOptimisticRow(
        tableId: 'app.messages',
        rowKey: 'message-1',
        phase: KalamRowSyncPhase.pending,
        pendingValuesJson: '{"id":"message-1","text":"hello"}',
      ),
    );

    final rowState = await database.select(database.kalamRowStates).getSingle();

    expect(action.status, KalamActionStatus.queued);
    expect(action.idempotencyKey, 'action-1');
    expect(action.createdAt, now);
    expect(rowState.actionId, 'action-1');
    expect(rowState.pendingValuesJson, contains('message-1'));
  });

  test('rolls back action and row state when optimistic work fails', () async {
    await expectLater(
      store.enqueue(
        const KalamActionDraft(
          id: 'action-1',
          accountKey: 'server/user-a',
          actionKey: 'messages.send',
          payloadJson: '{}',
        ),
        optimisticRow: const KalamOptimisticRow(
          tableId: 'app.messages',
          rowKey: 'message-1',
          phase: KalamRowSyncPhase.pending,
        ),
        applyOptimistic: () async => throw StateError('write failed'),
      ),
      throwsStateError,
    );

    expect(await database.select(database.kalamActions).get(), isEmpty);
    expect(await database.select(database.kalamRowStates).get(), isEmpty);
  });

  test('watches actions for one account only', () async {
    final first = store
        .watchActions('server/user-a')
        .firstWhere((actions) => actions.isNotEmpty);

    await store.enqueue(
      const KalamActionDraft(
        id: 'action-a',
        accountKey: 'server/user-a',
        actionKey: 'messages.send',
        payloadJson: '{}',
      ),
    );
    await store.enqueue(
      const KalamActionDraft(
        id: 'action-b',
        accountKey: 'server/user-b',
        actionKey: 'messages.send',
        payloadJson: '{}',
      ),
    );

    final actions = await first;

    expect(actions.map((action) => action.id), ['action-a']);
  });

  test('applies server work and advances its checkpoint atomically', () async {
    final applied = await store.applyAndCheckpoint(
      accountKey: 'server/user-a',
      subscriptionId: 'messages',
      seq: const SeqId(10),
      apply: () => database
          .into(database.kalamRowStates)
          .insert(
            KalamRowStatesCompanion.insert(
              accountKey: 'server/user-a',
              tableId: 'app.messages',
              rowKey: 'message-1',
              phase: 'synced',
              lastServerSeq: const Value('10'),
              updatedAt: now,
            ),
          ),
    );

    final checkpoint = await store.readCheckpoint(
      accountKey: 'server/user-a',
      subscriptionId: 'messages',
    );

    expect(applied, isTrue);
    expect(checkpoint?.seq, const SeqId(10));
    expect(await database.select(database.kalamRowStates).get(), hasLength(1));
  });

  test('does not apply an event older than the committed checkpoint', () async {
    var applyCount = 0;
    await store.applyAndCheckpoint(
      accountKey: 'server/user-a',
      subscriptionId: 'messages',
      seq: const SeqId(10),
      apply: () async => applyCount++,
    );

    final applied = await store.applyAndCheckpoint(
      accountKey: 'server/user-a',
      subscriptionId: 'messages',
      seq: const SeqId(9),
      apply: () async => applyCount++,
    );

    expect(applied, isFalse);
    expect(applyCount, 1);
  });

  test('does not advance checkpoint when server apply fails', () async {
    await expectLater(
      store.applyAndCheckpoint(
        accountKey: 'server/user-a',
        subscriptionId: 'messages',
        seq: const SeqId(10),
        apply: () async {
          await database
              .into(database.kalamRowStates)
              .insert(
                KalamRowStatesCompanion.insert(
                  accountKey: 'server/user-a',
                  tableId: 'app.messages',
                  rowKey: 'message-1',
                  phase: 'synced',
                  updatedAt: now,
                ),
              );
          throw StateError('decode failed');
        },
      ),
      throwsStateError,
    );

    expect(
      await store.readCheckpoint(
        accountKey: 'server/user-a',
        subscriptionId: 'messages',
      ),
      isNull,
    );
    expect(await database.select(database.kalamRowStates).get(), isEmpty);
  });
}
