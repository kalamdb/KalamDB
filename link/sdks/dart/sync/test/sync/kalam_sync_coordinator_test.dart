import 'dart:async';

import 'package:drift/native.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kalam_sync/drift.dart' hide isNotNull, isNull;
import 'package:kalam_sync/kalam_sync.dart';

final class FakeTransport implements KalamSyncTransport {
  final connections = StreamController<KalamTransportConnection>.broadcast();
  final streams = <String, StreamController<KalamRemoteBatch>>{};
  final calls = <({String id, String sql, SeqId? from, int? batchSize})>[];
  var pauseCount = 0;
  var resumeCount = 0;
  var ensureConnectedCount = 0;

  @override
  Stream<KalamTransportConnection> get connectionStates => connections.stream;

  @override
  Future<void> ensureConnected() async => ensureConnectedCount++;

  @override
  Stream<KalamRemoteBatch> subscribe({
    required String sql,
    required String subscriptionId,
    SeqId? from,
    int? batchSize,
  }) {
    calls.add((id: subscriptionId, sql: sql, from: from, batchSize: batchSize));
    return streams
        .putIfAbsent(subscriptionId, StreamController.broadcast)
        .stream;
  }

  @override
  Future<void> pause() async => pauseCount++;

  @override
  Future<void> resume() async => resumeCount++;

  Future<void> close() async {
    await connections.close();
    for (final stream in streams.values) {
      await stream.close();
    }
  }

  @override
  Future<void> dispose() => close();
}

void main() {
  late KalamSyncDatabase database;
  late KalamSyncStore store;
  late FakeTransport transport;
  late KalamSyncCoordinator coordinator;
  final now = DateTime.utc(2026, 8, 16, 12);

  setUp(() {
    database = KalamSyncDatabase(NativeDatabase.memory());
    store = KalamSyncStore(database, clock: () => now);
    transport = FakeTransport();
    coordinator = KalamSyncCoordinator(
      accountKey: 'server/user-a',
      store: store,
      transport: transport,
      actions: KalamActionRunner(
        accountKey: 'server/user-a',
        store: store,
        registry: KalamActionRegistry([]),
        clock: () => now,
      ),
    );
  });

  tearDown(() async {
    await coordinator.dispose();
    await transport.dispose();
    await database.close();
  });

  test('a custom consumer starts only where it is subscribed', () async {
    expect(transport.calls, isEmpty);

    final subscription = await coordinator.subscribe(
      KalamEventConsumer(
        id: 'conversation-1/events',
        sql: 'SELECT * FROM app.events WHERE conversation_id = ?',
        apply: (_) async {},
      ),
    );

    expect(transport.calls.single.id, 'conversation-1/events');
    await subscription.cancel();
    expect(subscription.isCancelled, isTrue);
  });

  test('resumes a consumer from its committed local checkpoint', () async {
    await store.applyAndCheckpoint(
      accountKey: 'server/user-a',
      subscriptionId: 'messages',
      seq: const SeqId(41),
      apply: () => null,
    );

    await coordinator.subscribe(
      KalamEventConsumer(
        id: 'messages',
        sql: 'SELECT * FROM app.messages',
        apply: (_) {},
      ),
    );

    expect(transport.calls.single.from, const SeqId(41));
  });

  test('commits an ordered batch before acknowledging its sequence', () async {
    final applied = <int>[];
    final acknowledged = Completer<void>();
    await coordinator.subscribe(
      KalamEventConsumer(
        id: 'messages',
        sql: 'SELECT * FROM app.messages',
        apply: (change) {
          applied.add(change.seq.value);
        },
      ),
    );
    final events = transport.streams['messages']!;

    events.add(
      _batch(
        [1, 1, 2],
        acknowledge: () async {
          expect(
            (await store.readCheckpoint(
              accountKey: 'server/user-a',
              subscriptionId: 'messages',
            ))?.seq,
            const SeqId(2),
            reason: 'transport progress must follow the SQLite commit',
          );
          acknowledged.complete();
        },
      ),
    );
    await acknowledged.future;

    expect(applied, [1, 2]);
    expect(
      (await store.readCheckpoint(
        accountKey: 'server/user-a',
        subscriptionId: 'messages',
      ))?.seq,
      const SeqId(2),
    );
  });

  test('stops a consumer when apply fails without skipping ahead', () async {
    var acknowledgementCount = 0;
    final failed = coordinator.states.firstWhere(
      (state) => state.phase == KalamSyncPhase.error,
    );
    await coordinator.subscribe(
      KalamEventConsumer(
        id: 'messages',
        sql: 'SELECT * FROM app.messages',
        apply: (change) {
          if (change.seq == const SeqId(1)) throw StateError('decode failed');
        },
      ),
    );

    transport.streams['messages']!.add(
      _batch([1, 2], acknowledge: () async => acknowledgementCount++),
    );
    await failed;

    expect(acknowledgementCount, 0);
    expect(
      await store.readCheckpoint(
        accountKey: 'server/user-a',
        subscriptionId: 'messages',
      ),
      isNull,
    );
  });

  test('serializes racing batches and acknowledges them in order', () async {
    final firstStarted = Completer<void>();
    final releaseFirst = Completer<void>();
    final allAcknowledged = Completer<void>();
    final applied = <int>[];
    final acknowledgements = <int>[];
    var concurrentApplies = 0;
    var maximumConcurrentApplies = 0;

    await coordinator.subscribe(
      KalamEventConsumer(
        id: 'messages',
        sql: 'SELECT * FROM app.messages',
        batchSize: 128,
        apply: (change) async {
          concurrentApplies++;
          maximumConcurrentApplies =
              maximumConcurrentApplies < concurrentApplies
              ? concurrentApplies
              : maximumConcurrentApplies;
          if (change.seq == const SeqId(1)) {
            firstStarted.complete();
            await releaseFirst.future;
          }
          applied.add(change.seq.value);
          concurrentApplies--;
        },
      ),
    );
    expect(transport.calls.single.batchSize, 128);

    void recordAck(int seq) {
      acknowledgements.add(seq);
      if (acknowledgements.length == 2) allAcknowledged.complete();
    }

    final events = transport.streams['messages']!;
    events
      ..add(_batch([1], acknowledge: () async => recordAck(1)))
      ..add(_batch([2], acknowledge: () async => recordAck(2)));
    await firstStarted.future;
    await Future<void>.delayed(Duration.zero);
    expect(applied, isEmpty, reason: 'the first transaction is still open');

    releaseFirst.complete();
    await allAcknowledged.future;

    expect(applied, [1, 2]);
    expect(acknowledgements, [1, 2]);
    expect(maximumConcurrentApplies, 1);
  });

  test('one table failure does not block another table checkpoint', () async {
    final healthyAcknowledged = Completer<void>();
    await coordinator.subscribe(
      KalamEventConsumer(
        id: 'messages',
        sql: 'SELECT * FROM app.messages',
        apply: (_) => throw StateError('bad message row'),
      ),
    );
    await coordinator.subscribe(
      KalamEventConsumer(
        id: 'conversations',
        sql: 'SELECT * FROM app.conversations',
        apply: (_) async {},
      ),
    );

    transport.streams['messages']!.add(_batch([3]));
    transport.streams['conversations']!.add(
      _batch([9], acknowledge: () async => healthyAcknowledged.complete()),
    );
    await healthyAcknowledged.future;

    expect(
      await store.readCheckpoint(
        accountKey: 'server/user-a',
        subscriptionId: 'messages',
      ),
      isNull,
    );
    expect(
      (await store.readCheckpoint(
        accountKey: 'server/user-a',
        subscriptionId: 'conversations',
      ))?.seq,
      const SeqId(9),
    );
  });

  test('applies a multi-row table batch in one SQLite transaction', () async {
    final acknowledged = Completer<void>();
    final binding = KalamTableBinding<Map<String, Object?>>(
      spec: KalamTableSpec(
        tableId: 'app.messages',
        keyColumn: 'id',
        subscriptionId: 'messages',
        mode: KalamSyncMode.replicaOnly,
        keyOf: (row) => row['id']! as String,
        encode: (row) => row,
        decode: (json) => json,
      ),
      accountKey: 'server/user-a',
      store: store,
    );
    final rowsReady = binding.watch().firstWhere((rows) => rows.length == 2);
    await coordinator.subscribe(
      binding.consumer(sql: 'SELECT * FROM app.messages', batchSize: 2),
    );

    transport.streams['messages']!.add(
      KalamRemoteBatch(
        changes: [_change(1), _change(2)],
        checkpoint: const SeqId(2),
        acknowledge: () async => acknowledged.complete(),
      ),
    );

    await acknowledged.future;
    expect(await rowsReady, hasLength(2));
  });

  test(
    'acknowledgement loss resumes from the already committed SQLite checkpoint',
    () async {
      final applyStarted = Completer<void>();
      final releaseApply = Completer<void>();
      final failed = coordinator.states.firstWhere(
        (state) => state.phase == KalamSyncPhase.error,
      );
      await coordinator.subscribe(
        KalamEventConsumer(
          id: 'messages',
          sql: 'SELECT * FROM app.messages',
          apply: (_) async {
            applyStarted.complete();
            await releaseApply.future;
          },
        ),
      );

      transport.streams['messages']!.add(
        _batch([
          5,
        ], acknowledge: () => throw StateError('connection lost before ack')),
      );
      await applyStarted.future;
      releaseApply.complete();
      await failed;

      expect(
        (await store.readCheckpoint(
          accountKey: 'server/user-a',
          subscriptionId: 'messages',
        ))?.seq,
        const SeqId(5),
      );

      transport.connections.add(KalamTransportConnection.connected);
      await _eventually(() => transport.calls.length == 2);

      expect(transport.calls.last.from, const SeqId(5));
    },
    timeout: const Timeout(Duration(seconds: 5)),
  );

  test(
    'disconnect during apply resumes from the completed transaction',
    () async {
      final applyStarted = Completer<void>();
      final releaseApply = Completer<void>();
      var acknowledgementCount = 0;
      await coordinator.subscribe(
        KalamEventConsumer(
          id: 'messages',
          sql: 'SELECT * FROM app.messages',
          apply: (_) async {
            applyStarted.complete();
            await releaseApply.future;
          },
        ),
      );
      transport.streams['messages']!.add(
        _batch([7], acknowledge: () async => acknowledgementCount++),
      );
      await applyStarted.future;

      final offline = coordinator.states.firstWhere(
        (state) => state.phase == KalamSyncPhase.offline,
      );
      transport.connections.add(KalamTransportConnection.disconnected);
      await Future<void>.delayed(Duration.zero);
      releaseApply.complete();
      await offline;

    expect(
      acknowledgementCount,
      0,
      reason: 'the disconnected transport cannot be acknowledged',
    );
      expect(
        (await store.readCheckpoint(
          accountKey: 'server/user-a',
          subscriptionId: 'messages',
        ))?.seq,
        const SeqId(7),
      );

      transport.connections.add(KalamTransportConnection.connected);
      await _eventually(() => transport.calls.length == 2);
      expect(transport.calls.last.from, const SeqId(7));
    },
    timeout: const Timeout(Duration(seconds: 5)),
  );

  test('connection and lifecycle changes are exposed as sync state', () async {
    transport.connections.add(KalamTransportConnection.connected);
    await coordinator.states.firstWhere(
      (state) => state.phase == KalamSyncPhase.live,
    );

    transport.connections.add(KalamTransportConnection.disconnected);
    await coordinator.states.firstWhere(
      (state) => state.phase == KalamSyncPhase.offline,
    );

    await coordinator.pause();
    expect(coordinator.state.phase, KalamSyncPhase.paused);
    expect(transport.pauseCount, 1);

    await coordinator.resume();
    expect(transport.resumeCount, 1);
  });

  test(
    'a late state listener immediately receives the current state',
    () async {
      transport.connections.add(KalamTransportConnection.connected);
      await coordinator.states.firstWhere(
        (state) => state.phase == KalamSyncPhase.live,
      );

      expect((await coordinator.states.first).phase, KalamSyncPhase.live);
    },
  );

  test('pending actions wake the connection without a consumer', () async {
    final action = await store.enqueue(
      const KalamActionDraft(
        id: 'action-1',
        accountKey: 'server/user-a',
        actionKey: 'messages.send',
        payloadJson: '{}',
      ),
    );

    await coordinator.store
        .watchActions('server/user-a')
        .firstWhere(
          (records) => records.any((record) => record.id == action.id),
        );
    await Future<void>.delayed(Duration.zero);

    expect(transport.ensureConnectedCount, 1);
  });
}

KalamRemoteChange _change(int seq) {
  return KalamRemoteChange(
    kind: KalamChangeKind.insert,
    seq: SeqId(seq),
    row: {'id': 'message-$seq', '_seq': seq},
  );
}

KalamRemoteBatch _batch(
  List<int> sequences, {
  Future<void> Function()? acknowledge,
}) {
  return KalamRemoteBatch(
    changes: [for (final seq in sequences) _change(seq)],
    checkpoint: sequences.isEmpty ? null : SeqId(sequences.last),
    acknowledge: acknowledge ?? () async {},
  );
}

Future<void> _eventually(bool Function() condition) async {
  final deadline = DateTime.now().add(const Duration(seconds: 2));
  while (!condition()) {
    if (DateTime.now().isAfter(deadline)) {
      throw TimeoutException('condition was not reached');
    }
    await Future<void>.delayed(const Duration(milliseconds: 5));
  }
}
