import 'package:drift/native.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kalam_sync/kalam_sync.dart';
import 'package:kalam_sync/drift.dart';

void main() {
  late KalamSyncDatabase database;
  late KalamSyncStore store;
  setUp(() {
    database = KalamSyncDatabase(NativeDatabase.memory());
    store = KalamSyncStore(database);
  });
  tearDown(() => database.close());

  test('identical retry does not repeat optimistic side effects', () async {
    const draft = KalamActionDraft(
      id: 'retry',
      accountKey: 'a',
      actionKey: 'send',
      payloadJson: '{}',
    );
    var applies = 0;
    final first = await store.enqueue(
      draft,
      applyOptimistic: () async {
        applies++;
      },
    );
    final retry = await store.enqueue(
      draft,
      applyOptimistic: () async {
        applies++;
      },
    );
    expect(retry.id, first.id);
    expect(applies, 1);
  });

  test(
    'seed preserves an existing lower checkpoint rather than fast-forwarding it',
    () async {
      await store.seedCheckpointIfAbsent(
        accountKey: 'a',
        subscriptionId: 's',
        seq: VersionId.parse('4'),
      );
      expect(
        await store.seedCheckpointIfAbsent(
          accountKey: 'a',
          subscriptionId: 's',
          seq: VersionId.parse('99'),
        ),
        isFalse,
      );
      expect(
        (await store.readCheckpoint(accountKey: 'a', subscriptionId: 's'))!.seq,
        VersionId.parse('4'),
      );
    },
  );

  test('duplicate id with different payload must be rejected', () async {
    await store.enqueue(
      const KalamActionDraft(
        id: 'stable-id',
        accountKey: 'account-a',
        actionKey: 'send',
        payloadJson: '{"text":"one"}',
      ),
    );
    await expectLater(
      store.enqueue(
        const KalamActionDraft(
          id: 'stable-id',
          accountKey: 'account-a',
          actionKey: 'send',
          payloadJson: '{"text":"two"}',
        ),
      ),
      throwsA(anything),
    );
  });

  test('shared database cannot return another account action', () async {
    await store.enqueue(
      const KalamActionDraft(
        id: 'mailbox:1',
        accountKey: 'account-a',
        actionKey: 'project',
        payloadJson: '{}',
      ),
    );
    await expectLater(
      store.enqueue(
        const KalamActionDraft(
          id: 'mailbox:1',
          accountKey: 'account-b',
          actionKey: 'project',
          payloadJson: '{}',
        ),
      ),
      throwsStateError,
    );
  });

  test('concurrent checkpoint seeds allow only one insertion', () async {
    final results = await Future.wait([
      store.seedCheckpointIfAbsent(
        accountKey: 'a',
        subscriptionId: 's',
        seq: VersionId.parse('4'),
      ),
      store.seedCheckpointIfAbsent(
        accountKey: 'a',
        subscriptionId: 's',
        seq: VersionId.parse('9'),
      ),
    ]);
    expect(results.where((inserted) => inserted).length, 1);
  });
}
