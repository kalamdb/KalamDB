import 'dart:async';

import 'package:drift/native.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kalam_sync/drift.dart' hide isNotNull, isNull;
import 'package:kalam_sync/kalam_sync.dart';

final class _WrapperTransport implements KalamSyncTransport {
  bool failOnDispose = false;
  @override
  Stream<KalamTransportConnection> get connectionStates => const Stream.empty();

  @override
  Stream<KalamRemoteBatch> subscribe({
    required String sql,
    required String subscriptionId,
    VersionId? from,
    int? batchSize,
    List<Object?>? params,
  }) => const Stream.empty();

  @override
  Future<void> ensureConnected() async {}

  @override
  Future<void> pause() async {}

  @override
  Future<void> resume() async {}

  @override
  Future<void> dispose() async {
    if (failOnDispose) throw StateError('transport cleanup failed');
  }
}

void main() {
  test('owned database closes even when transport disposal fails', () async {
    final database = _TrackingDatabase();
    final transport = _WrapperTransport()..failOnDispose = true;
    final kalam = Kalam.fromComponents(
      identity: const KalamAccountIdentity(
        serverUrl: 'http://db',
        subject: 'a',
      ),
      database: database,
      transport: transport,
    );
    await expectLater(kalam.dispose(), throwsStateError);
    expect(database.closed, isTrue);
  });

  test(
    'wrapper transports do not register DML without an explicit client',
    () async {
      final database = KalamSyncDatabase(NativeDatabase.memory());
      addTearDown(database.close);
      final kalam = Kalam.fromComponents(
        identity: const KalamAccountIdentity(
          serverUrl: 'http://localhost:2900',
          subject: 'admin',
          namespace: 'app',
        ),
        database: database,
        transport: _WrapperTransport(),
      );
      expect(
        () => kalam.actions.registry['kalam.dml'],
        throwsA(
          isA<StateError>().having(
            (error) => error.toString(),
            'message',
            contains('kalam.dml'),
          ),
        ),
      );
    },
  );

  test('table() projects into caller-owned local storage', () async {
    final database = KalamSyncDatabase(NativeDatabase.memory());
    addTearDown(database.close);
    final localRows = <Map<String, Object?>>[];
    final kalam = Kalam.fromComponents(
      identity: const KalamAccountIdentity(
        serverUrl: 'http://localhost:2900',
        subject: 'admin',
        namespace: 'app',
      ),
      database: database,
      transport: _WrapperTransport(),
    );
    addTearDown(kalam.dispose);

    final messages = kalam.table(
      KalamTableSpec<Map<String, Object?>>(
        tableId: 'public.messages',
        keyColumn: 'id',
        mode: KalamSyncMode.replicaOnly,
        keyOf: (row) => row['id']! as String,
        encode: (row) => row,
        decode: (json) => json,
      ),
      watchLocal: () async* {
        yield List.of(localRows);
      },
      upsertLocal: (row) async {
        localRows
          ..removeWhere((existing) => existing['id'] == row['id'])
          ..add(row);
      },
      deleteLocal: (rowKey) async {
        localRows.removeWhere((row) => row['id'] == rowKey);
      },
    );

    await messages.applyServerChange(
      const KalamChange(
        kind: KalamChangeKind.insert,
        rowKey: 'message-1',
        row: {'id': 'message-1', 'text': 'projected'},
        seq: VersionId(1),
      ),
    );

    expect(localRows, [
      {'id': 'message-1', 'text': 'projected'},
    ]);
    expect(await database.select(database.kalamCachedRows).get(), isEmpty);
  });
}

class _TrackingDatabase extends KalamSyncDatabase {
  _TrackingDatabase() : super(NativeDatabase.memory());
  bool closed = false;
  @override
  Future<void> close() async {
    closed = true;
    await super.close();
  }
}
