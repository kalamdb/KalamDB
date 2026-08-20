import 'dart:async';

import 'package:drift/native.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kalam_sync/drift.dart' hide isNotNull, isNull;
import 'package:kalam_sync/kalam_sync.dart';

final class _CatchUpTransport implements KalamSyncTransport {
  final pages = <String, List<KalamRemoteBatch>>{};
  final calls =
      <({String id, String sql, SeqId? from, int? batchSize})>[];

  @override
  Stream<KalamTransportConnection> get connectionStates =>
      const Stream.empty();

  @override
  Stream<KalamRemoteBatch> subscribe({
    required String sql,
    required String subscriptionId,
    SeqId? from,
    int? batchSize,
    List<Object?>? params,
  }) {
    calls.add((
      id: subscriptionId,
      sql: sql,
      from: from,
      batchSize: batchSize,
    ));
    final queued = pages[subscriptionId] ?? const <KalamRemoteBatch>[];
    return Stream<KalamRemoteBatch>.fromIterable(queued);
  }

  @override
  Future<void> ensureConnected() async {}

  @override
  Future<void> pause() async {}

  @override
  Future<void> resume() async {}

  @override
  Future<void> dispose() async {}
}

void main() {
  test('catch-up uses the live query from the committed checkpoint', () async {
    final database = KalamSyncDatabase(NativeDatabase.memory());
    addTearDown(database.close);
    final store = KalamSyncStore(database);
    await store.applyAndCheckpoint(
      accountKey: const KalamAccountIdentity(
        serverUrl: 'https://db.example.com',
        subject: 'user-a',
      ).accountKey,
      subscriptionId: 'messages',
      seq: const SeqId(10),
      apply: () => null,
    );

    final transport = _CatchUpTransport()
      ..pages['messages'] = [
        KalamRemoteBatch(
          changes: [
            const KalamRemoteChange(
              kind: KalamChangeKind.insert,
              seq: SeqId(11),
              row: {'id': 'message-11'},
            ),
            const KalamRemoteChange(
              kind: KalamChangeKind.insert,
              seq: SeqId(12),
              row: {'id': 'message-12'},
            ),
          ],
          checkpoint: const SeqId(12),
          acknowledge: () async {},
        ),
      ];

    final applied = <int>[];
    final result = await Kalam.catchUp(
      url: 'https://db.example.com',
      subject: 'user-a',
      database: database,
      transport: transport,
      rowLimit: 2,
      consumers: [
        KalamEventConsumer(
          id: 'messages',
          sql: 'SELECT * FROM public.messages',
          apply: (change) async => applied.add(change.seq.value),
        ),
      ],
    );

    expect(transport.calls.single.sql, 'SELECT * FROM public.messages');
    expect(transport.calls.single.from, const SeqId(10));
    expect(transport.calls.single.batchSize, 2);
    expect(applied, [11, 12]);
    expect(result.appliedCount, 2);
    expect(result.hasMore, isTrue);
    expect(
      (await store.readCheckpoint(
        accountKey: transport.calls.single.id == 'messages'
            ? const KalamAccountIdentity(
                serverUrl: 'https://db.example.com',
                subject: 'user-a',
              ).accountKey
            : '',
        subscriptionId: 'messages',
      ))?.seq,
      const SeqId(12),
    );
  });

  test('later live subscribe resumes from the catch-up checkpoint', () async {
    final database = KalamSyncDatabase(NativeDatabase.memory());
    addTearDown(database.close);
    final catchUp = _CatchUpTransport()
      ..pages['messages'] = [
        KalamRemoteBatch(
          changes: [
            const KalamRemoteChange(
              kind: KalamChangeKind.insert,
              seq: SeqId(3),
              row: {'id': 'message-3'},
            ),
          ],
          checkpoint: const SeqId(3),
          acknowledge: () async {},
        ),
      ];

    await Kalam.catchUp(
      url: 'https://db.example.com',
      subject: 'user-a',
      database: database,
      transport: catchUp,
      consumers: [
        KalamEventConsumer(
          id: 'messages',
          sql: 'SELECT * FROM public.messages',
          apply: (_) async {},
        ),
      ],
    );

    final live = _CatchUpTransport();
    final kalam = Kalam.fromComponents(
      identity: const KalamAccountIdentity(
        serverUrl: 'https://db.example.com',
        subject: 'user-a',
      ),
      database: database,
      transport: live,
    );
    addTearDown(kalam.dispose);

    await kalam.subscribe(
      KalamEventConsumer(
        id: 'messages',
        sql: 'SELECT * FROM public.messages',
        apply: (_) {},
      ),
    );

    expect(live.calls.single.from, const SeqId(3));
  });
}
