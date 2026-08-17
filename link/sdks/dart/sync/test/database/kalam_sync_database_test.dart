import 'dart:io';

import 'package:drift/native.dart';
import 'package:kalam_sync/drift.dart' hide isNotNull, isNull;
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('stores durable actions with retry metadata', () async {
    final database = KalamSyncDatabase(NativeDatabase.memory());
    addTearDown(database.close);

    final now = DateTime.utc(2026, 8, 16);
    await database
        .into(database.kalamActions)
        .insert(
          KalamActionsCompanion.insert(
            id: 'action-1',
            accountKey: 'server/user-a',
            actionKey: 'messages.send@1',
            payloadJson: '{"text":"hello"}',
            status: 'retryScheduled',
            orderingKey: const Value('conversation-1'),
            attemptCount: const Value(2),
            nextAttemptAt: Value(now.add(const Duration(seconds: 5))),
            createdAt: now,
            updatedAt: now,
          ),
        );

    final action = await database.select(database.kalamActions).getSingle();

    expect(action.id, 'action-1');
    expect(action.actionKey, 'messages.send@1');
    expect(action.attemptCount, 2);
    expect(action.orderingKey, 'conversation-1');
  });

  test('isolates checkpoints and row states by account', () async {
    final database = KalamSyncDatabase(NativeDatabase.memory());
    addTearDown(database.close);

    final now = DateTime.utc(2026, 8, 16);
    await database.batch((batch) {
      batch.insertAll(database.kalamCheckpoints, [
        KalamCheckpointsCompanion.insert(
          accountKey: 'server/user-a',
          subscriptionId: 'messages',
          seq: '10',
          updatedAt: now,
        ),
        KalamCheckpointsCompanion.insert(
          accountKey: 'server/user-b',
          subscriptionId: 'messages',
          seq: '3',
          updatedAt: now,
        ),
      ]);
      batch.insertAll(database.kalamRowStates, [
        KalamRowStatesCompanion.insert(
          accountKey: 'server/user-a',
          tableId: 'app.messages',
          rowKey: 'message-1',
          phase: 'pending',
          updatedAt: now,
        ),
        KalamRowStatesCompanion.insert(
          accountKey: 'server/user-b',
          tableId: 'app.messages',
          rowKey: 'message-1',
          phase: 'synced',
          updatedAt: now,
        ),
      ]);
    });

    final checkpoints = await database.select(database.kalamCheckpoints).get();
    final states = await database.select(database.kalamRowStates).get();

    expect(checkpoints, hasLength(2));
    expect(states, hasLength(2));
    expect(
      states.map((state) => state.phase),
      containsAll(['pending', 'synced']),
    );
  });

  test('persists checkpoints and action steps across reopen', () async {
    final directory = await Directory.systemTemp.createTemp('kalam-sync-test-');
    addTearDown(() => directory.delete(recursive: true));
    final file = File('${directory.path}/sync.sqlite');
    final now = DateTime.utc(2026, 8, 16);

    var database = KalamSyncDatabase(NativeDatabase(file));
    await database
        .into(database.kalamCheckpoints)
        .insert(
          KalamCheckpointsCompanion.insert(
            accountKey: 'server/user-a',
            subscriptionId: 'messages',
            seq: '44',
            updatedAt: now,
          ),
        );
    await database
        .into(database.kalamActionSteps)
        .insert(
          KalamActionStepsCompanion.insert(
            actionId: 'action-1',
            name: 'upload',
            status: 'succeeded',
            resultJson: const Value('{"fileId":"file-1"}'),
            updatedAt: now,
          ),
        );
    await database.close();

    database = KalamSyncDatabase(NativeDatabase(file));
    addTearDown(database.close);

    final checkpoint = await database
        .select(database.kalamCheckpoints)
        .getSingle();
    final step = await database.select(database.kalamActionSteps).getSingle();

    expect(checkpoint.seq, '44');
    expect(step.resultJson, '{"fileId":"file-1"}');
  });
}
