import 'dart:async';

import 'package:drift/native.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kalam_sync/drift.dart' hide isNotNull, isNull;
import 'package:kalam_sync/kalam_sync.dart';

final class _CatchUpTransport implements KalamSyncTransport {
  final pages = <String, List<KalamRemoteBatch>>{};
  final calls = <({String id, String sql, VersionId? from, int? batchSize})>[];

  @override
  Stream<KalamTransportConnection> get connectionStates => const Stream.empty();

  @override
  Stream<KalamRemoteBatch> subscribe({
    required String sql,
    required String subscriptionId,
    VersionId? from,
    int? batchSize,
    List<Object?>? params,
  }) {
    calls.add((id: subscriptionId, sql: sql, from: from, batchSize: batchSize));
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
  test(
    'timeout waits for an in-flight durable apply before returning',
    () async {
      final database = KalamSyncDatabase(NativeDatabase.memory());
      addTearDown(database.close);
      final entered = Completer<void>();
      final release = Completer<void>();
      final transport = _OpenCatchUpTransport(
        replay: true,
        emitBoundary: false,
      );
      var completed = false;
      final pending =
          Kalam.catchUp(
            url: 'https://db.example.com',
            subject: 'user-a',
            database: database,
            transport: transport,
            timeout: const Duration(milliseconds: 100),
            consumers: [
              KalamEventConsumer(
                id: 'messages',
                sql: 'SELECT * FROM public.messages',
                apply: (_) async {
                  if (!entered.isCompleted) entered.complete();
                  await release.future;
                },
              ),
            ],
          ).then((result) {
            completed = true;
            return result;
          });
      await entered.future;
      await Future<void>.delayed(const Duration(milliseconds: 150));
      expect(completed, isFalse);
      release.complete();
      final result = await pending;
      expect(result.timedOut, isTrue);
      final store = KalamSyncStore(database);
      const identity = KalamAccountIdentity(
        serverUrl: 'https://db.example.com',
        subject: 'user-a',
      );
      expect(
        (await store.readCheckpoint(
          accountKey: identity.accountKey,
          subscriptionId: 'messages',
        ))?.seq,
        const VersionId(1),
      );
    },
  );
  test(
    'replay does not stop at a snapshot marker preceding replay events',
    () async {
      final database = KalamSyncDatabase(NativeDatabase.memory());
      addTearDown(database.close);
      final transport = _OpenCatchUpTransport(replay: true, markerFirst: true);
      final applied = <int>[];
      final result = await Kalam.catchUp(
        url: 'https://db.example.com',
        subject: 'user-a',
        database: database,
        transport: transport,
        timeout: const Duration(milliseconds: 100),
        consumers: [
          KalamEventConsumer(
            id: 'messages',
            sql: 'SELECT * FROM public.messages',
            apply: (change) => applied.add(change.seq.value),
          ),
        ],
      );
      expect(applied, [1, 2]);
      expect(result.timedOut, isTrue);
    },
  );
  test('catch-up reports timeout and cancels an unfinished snapshot', () async {
    final database = KalamSyncDatabase(NativeDatabase.memory());
    addTearDown(database.close);
    final transport = _OpenCatchUpTransport(emitBoundary: false);
    final result = await Kalam.catchUp(
      url: 'https://db.example.com',
      subject: 'user-a',
      database: database,
      transport: transport,
      timeout: const Duration(milliseconds: 100),
      consumers: [
        KalamEventConsumer(
          id: 'messages',
          sql: 'SELECT * FROM public.messages',
          apply: (_) {},
        ),
      ],
    );
    expect(result.timedOut, isTrue);
    expect(transport.cancelled, 1);
  });
  test(
    'catch-up stops on an empty completed snapshot and reaches later consumers',
    () async {
      final database = KalamSyncDatabase(NativeDatabase.memory());
      addTearDown(database.close);
      final transport = _OpenCatchUpTransport();
      final seen = <String>[];
      final result = await Kalam.catchUp(
        url: 'https://db.example.com',
        subject: 'user-a',
        database: database,
        transport: transport,
        mode: KalamCatchUpMode.snapshot,
        timeout: const Duration(milliseconds: 300),
        consumers: [
          for (final id in ['messages', 'notifications', 'updates'])
            KalamEventConsumer(
              id: id,
              sql: 'SELECT * FROM public.$id',
              apply: (_) {},
            ),
        ],
      );
      seen.addAll(transport.calls);
      expect(seen, ['messages', 'notifications', 'updates']);
      expect(result.timedOut, isFalse);
      expect(transport.cancelled, 3);
    },
  );

  test(
    'catch-up waits for snapshot completion after a short replay batch',
    () async {
      final database = KalamSyncDatabase(NativeDatabase.memory());
      addTearDown(database.close);
      final transport = _OpenCatchUpTransport(replay: true);
      final applied = <int>[];
      final result = await Kalam.catchUp(
        url: 'https://db.example.com',
        subject: 'user-a',
        database: database,
        transport: transport,
        mode: KalamCatchUpMode.snapshot,
        timeout: const Duration(seconds: 1),
        consumers: [
          KalamEventConsumer(
            id: 'messages',
            sql: 'SELECT * FROM public.messages',
            apply: (change) => applied.add(change.seq.value),
          ),
        ],
      );
      expect(applied, [1, 2]);
      expect(result.appliedCount, 2);
      expect(result.timedOut, isFalse);
    },
  );
  test(
    'snapshot catch-up fetches without a replay cursor and preserves its checkpoint',
    () async {
      final database = KalamSyncDatabase(NativeDatabase.memory());
      addTearDown(database.close);
      final store = KalamSyncStore(database);
      const identity = KalamAccountIdentity(
        serverUrl: 'https://db.example.com',
        subject: 'user-a',
      );
      await store.applyAndCheckpoint(
        accountKey: identity.accountKey,
        subscriptionId: 'messages',
        seq: const VersionId(10),
        apply: () => null,
      );
      final transport = _CatchUpTransport()
        ..pages['messages'] = [
          KalamRemoteBatch(
            changes: [
              const KalamRemoteChange(
                kind: KalamChangeKind.insert,
                seq: VersionId(11),
                row: {'id': 'new'},
              ),
            ],
            checkpoint: const VersionId(11),
            snapshotComplete: true,
            acknowledge: () async {},
          ),
        ];
      final applied = <int>[];
      final result = await Kalam.catchUp(
        url: 'https://db.example.com',
        subject: 'user-a',
        database: database,
        transport: transport,
        mode: KalamCatchUpMode.snapshot,
        consumers: [
          KalamEventConsumer(
            id: 'messages',
            sql: 'SELECT * FROM public.messages',
            apply: (change) => applied.add(change.seq.value),
          ),
        ],
      );
      expect(transport.calls.single.from, isNull);
      expect(applied, [11]);
      expect(result.timedOut, isFalse);
    },
  );
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
      seq: const VersionId(10),
      apply: () => null,
    );

    final transport = _CatchUpTransport()
      ..pages['messages'] = [
        KalamRemoteBatch(
          changes: [
            const KalamRemoteChange(
              kind: KalamChangeKind.insert,
              seq: VersionId(11),
              row: {'id': 'message-11'},
            ),
            const KalamRemoteChange(
              kind: KalamChangeKind.insert,
              seq: VersionId(12),
              row: {'id': 'message-12'},
            ),
          ],
          checkpoint: const VersionId(12),
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
    expect(transport.calls.single.from, const VersionId(10));
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
      const VersionId(12),
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
              seq: VersionId(3),
              row: {'id': 'message-3'},
            ),
          ],
          checkpoint: const VersionId(3),
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

    expect(live.calls.single.from, const VersionId(3));
  });
}

final class _OpenCatchUpTransport implements KalamSyncTransport {
  _OpenCatchUpTransport({
    this.replay = false,
    this.emitBoundary = true,
    this.markerFirst = false,
  });
  final bool replay;
  final bool emitBoundary;
  final bool markerFirst;
  final calls = <String>[];
  int cancelled = 0;
  @override
  Stream<KalamRemoteBatch> subscribe({
    required String sql,
    required String subscriptionId,
    VersionId? from,
    int? batchSize,
    List<Object?>? params,
  }) {
    calls.add(subscriptionId);
    late StreamController<KalamRemoteBatch> controller;
    controller = StreamController(
      onListen: () {
        if (markerFirst) {
          controller.add(
            KalamRemoteBatch(
              changes: [],
              checkpoint: null,
              acknowledge: () async {},
              snapshotComplete: true,
            ),
          );
        }
        if (replay) {
          for (final seq in [1, 2]) {
            controller.add(
              KalamRemoteBatch(
                changes: [
                  KalamRemoteChange(
                    kind: KalamChangeKind.insert,
                    seq: VersionId(seq),
                    row: {'id': '$seq'},
                  ),
                ],
                checkpoint: VersionId(seq),
                acknowledge: () async {},
              ),
            );
          }
        }
        if (emitBoundary) {
          controller.add(
            KalamRemoteBatch(
              changes: [],
              checkpoint: null,
              acknowledge: () async {},
              snapshotComplete: true,
            ),
          );
        }
      },
      onCancel: () {
        cancelled++;
      },
    );
    return controller.stream;
  }

  @override
  Stream<KalamTransportConnection> get connectionStates => const Stream.empty();
  @override
  Future<void> ensureConnected() async {}
  @override
  Future<void> pause() async {}
  @override
  Future<void> resume() async {}
  @override
  Future<void> dispose() async {}
}
