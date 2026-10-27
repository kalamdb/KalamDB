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

  test('duplicate id with different payload must be rejected', () async {
    await store.enqueue(const KalamActionDraft(
      id: 'stable-id', accountKey: 'account-a', actionKey: 'send', payloadJson: '{"text":"one"}'));
    await expectLater(store.enqueue(const KalamActionDraft(
      id: 'stable-id', accountKey: 'account-a', actionKey: 'send', payloadJson: '{"text":"two"}')),
      throwsA(anything));
  });

  test('shared database cannot return another account action', () async {
    await store.enqueue(const KalamActionDraft(
      id: 'mailbox:1', accountKey: 'account-a', actionKey: 'project', payloadJson: '{}'));
    final result = await store.enqueue(const KalamActionDraft(
      id: 'mailbox:1', accountKey: 'account-b', actionKey: 'project', payloadJson: '{}'));
    expect(result.accountKey, 'account-b');
  });

  test('concurrent checkpoint seeds allow only one insertion', () async {
    final results = await Future.wait([
      store.seedCheckpointIfAbsent(accountKey:'a', subscriptionId:'s', seq:VersionId.parse('4')),
      store.seedCheckpointIfAbsent(accountKey:'a', subscriptionId:'s', seq:VersionId.parse('9')),
    ]);
    expect(results.where((inserted) => inserted).length, 1);
  });
}
