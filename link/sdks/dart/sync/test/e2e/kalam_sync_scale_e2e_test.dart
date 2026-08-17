import 'dart:async';

import 'package:drift/native.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kalam_sync/drift.dart' hide isNotNull, isNull;
import 'package:kalam_sync/kalam_sync.dart';

import '../../../link/test/e2e/helpers.dart';

final class _LargeRow {
  const _LargeRow(this.id, this.value);

  final int id;
  final String value;
}

final class _CountingTransport implements KalamSyncTransport {
  _CountingTransport(this.delegate);

  final KalamLinkTransport delegate;
  final Map<String, List<int>> deliveredBatchSizes = {};
  final Map<String, List<SeqId>> acknowledged = {};

  @override
  Stream<KalamTransportConnection> get connectionStates =>
      delegate.connectionStates;

  @override
  Stream<KalamRemoteBatch> subscribe({
    required String sql,
    required String subscriptionId,
    SeqId? from,
    int? batchSize,
  }) {
    return delegate
        .subscribe(
          sql: sql,
          subscriptionId: subscriptionId,
          from: from,
          batchSize: batchSize,
        )
        .map((batch) {
          if (batch.checkpoint != null) {
            deliveredBatchSizes
                .putIfAbsent(subscriptionId, () => [])
                .add(batch.changes.length);
          }
          return KalamRemoteBatch(
            changes: batch.changes,
            checkpoint: batch.checkpoint,
            acknowledge: () async {
              await batch.acknowledge();
              final checkpoint = batch.checkpoint;
              if (checkpoint != null) {
                acknowledged
                    .putIfAbsent(subscriptionId, () => [])
                    .add(checkpoint);
              }
            },
          );
        });
  }

  @override
  Future<void> ensureConnected() => delegate.ensureConnected();

  @override
  Future<void> pause() => delegate.pause();

  @override
  Future<void> resume() => delegate.resume();

  @override
  Future<void> dispose() => delegate.dispose();
}

void main() {
  test(
    'syncs 10k rows in acknowledged SQLite batches',
    () async {
      await ensureSdkReady();
      final setup = await connectJwtClient();
      final namespace = uniqueName('sync_scale');
      final table = '$namespace.items';
      await setup.query('CREATE NAMESPACE IF NOT EXISTS $namespace');
      final created = await setup.query(
        'CREATE TABLE $table (id BIGINT PRIMARY KEY, value TEXT NOT NULL)',
      );
      expect(created.success, isTrue, reason: created.error?.toString());
      addTearDown(() async {
        await setup.query('DROP TABLE IF EXISTS $table');
        await setup.dispose();
      });

      const rowCount = 10000;
      const insertSize = 500;
      for (var start = 0; start < rowCount; start += insertSize) {
        final values = [
          for (var id = start; id < start + insertSize; id++)
            "($id, 'value-$id')",
        ].join(', ');
        final inserted = await setup.query(
          'INSERT INTO $table (id, value) VALUES $values',
        );
        expect(inserted.success, isTrue, reason: inserted.error?.toString());
      }

      final link = await KalamLinkTransport.connect(
        url: serverUrl,
        authProvider: () async => Auth.basic(adminUser, adminPass),
      );
      final transport = _CountingTransport(link);
      final database = KalamSyncDatabase(NativeDatabase.memory());
      final kalam = Kalam.fromComponents(
        identity: KalamAccountIdentity(
          serverUrl: serverUrl,
          subject: adminUser,
          namespace: namespace,
        ),
        database: database,
        transport: transport,
      );
      addTearDown(kalam.dispose);
      final subscriptionId = '$table/all';
      final binding = kalam.table(
        KalamTableSpec<_LargeRow>(
          tableId: table,
          keyColumn: 'id',
          subscriptionId: subscriptionId,
          mode: KalamSyncMode.replicaOnly,
          keyOf: (row) => row.id.toString(),
          encode: (row) => {'id': row.id, 'value': row.value},
          decode: (json) => _LargeRow(
            int.parse(json['id']!.toString()),
            json['value']! as String,
          ),
        ),
      );

      final stopwatch = Stopwatch()..start();
      final allRows = binding.watch().firstWhere(
        (rows) => rows.length == rowCount,
      );
      final subscription = await kalam.subscribe(
        binding.consumer(sql: 'SELECT * FROM $table', batchSize: 250),
      );
      addTearDown(subscription.cancel);
      final rows = await _withSyncErrors(
        kalam,
        allRows,
      ).timeout(const Duration(seconds: 90));
      stopwatch.stop();

      final sizes = transport.deliveredBatchSizes[subscriptionId] ?? const [];
      await waitForCondition(
        () =>
            (transport.acknowledged[subscriptionId]?.length ?? 0) ==
            sizes.length,
        timeout: const Duration(seconds: 10),
        poll: const Duration(milliseconds: 10),
      );
      final acknowledgements = transport.acknowledged[subscriptionId]!;
      final checkpoint = await kalam.store.readCheckpoint(
        accountKey: kalam.identity.accountKey,
        subscriptionId: subscriptionId,
      );
      // ignore: avoid_print
      print(
        'kalam_sync 10k snapshot: ${stopwatch.elapsedMilliseconds / 1000}s, '
        '${sizes.length} batches',
      );

      expect(rows.map((row) => row.id).toSet(), hasLength(rowCount));
      expect(sizes.length, greaterThan(1));
      expect(sizes.fold<int>(0, (sum, size) => sum + size), rowCount);
      expect(acknowledgements, hasLength(sizes.length));
      expect(checkpoint?.seq, acknowledgements.last);
    },
    skip: skipIfNoIntegration,
    timeout: const Timeout(Duration(seconds: 120)),
  );

  test(
    'syncs and checkpoints three tables independently on one connection',
    () async {
      await ensureSdkReady();
      final setup = await connectJwtClient();
      final namespace = uniqueName('sync_multi');
      final tables = [
        '$namespace.conversations',
        '$namespace.messages',
        '$namespace.notifications',
      ];
      await setup.query('CREATE NAMESPACE IF NOT EXISTS $namespace');
      for (final table in tables) {
        final created = await setup.query(
          'CREATE TABLE $table (id BIGINT PRIMARY KEY, value TEXT NOT NULL)',
        );
        expect(created.success, isTrue, reason: created.error?.toString());
        await setup.query(
          "INSERT INTO $table (id, value) VALUES (1, 'initial')",
        );
      }
      addTearDown(() async {
        for (final table in tables) {
          await setup.query('DROP TABLE IF EXISTS $table');
        }
        await setup.dispose();
      });

      final link = await KalamLinkTransport.connect(
        url: serverUrl,
        authProvider: () async => Auth.basic(adminUser, adminPass),
      );
      final transport = _CountingTransport(link);
      final kalam = Kalam.fromComponents(
        identity: KalamAccountIdentity(
          serverUrl: serverUrl,
          subject: adminUser,
          namespace: namespace,
        ),
        database: KalamSyncDatabase(NativeDatabase.memory()),
        transport: transport,
      );
      addTearDown(kalam.dispose);

      final bindings = [
        for (final table in tables)
          kalam.table(
            KalamTableSpec<_LargeRow>(
              tableId: table,
              keyColumn: 'id',
              subscriptionId: '$table/all',
              mode: KalamSyncMode.replicaOnly,
              keyOf: (row) => row.id.toString(),
              encode: (row) => {'id': row.id, 'value': row.value},
              decode: (json) => _LargeRow(
                int.parse(json['id']!.toString()),
                json['value']! as String,
              ),
            ),
          ),
      ];
      final subscriptions = <KalamSyncSubscription>[];
      final appliedChanges = <String, int>{};
      for (var index = 0; index < tables.length; index++) {
        final table = tables[index];
        final initial = bindings[index].watch().firstWhere(
          (rows) => rows.length == 1,
        );
        final consumer = bindings[index].consumer(
          sql: 'SELECT * FROM $table',
          batchSize: 2,
        );
        subscriptions.add(
          await kalam.subscribe(
            KalamEventConsumer(
              id: consumer.id,
              sql: consumer.sql,
              batchSize: consumer.batchSize,
              apply: (change) async {
                appliedChanges[table] = (appliedChanges[table] ?? 0) + 1;
                await consumer.apply(change);
              },
            ),
          ),
        );
        await _withSyncErrors(
          kalam,
          initial,
        ).timeout(const Duration(seconds: 15));
      }
      addTearDown(() async {
        for (final subscription in subscriptions) {
          await subscription.cancel();
        }
      });

      final completed = [
        for (final binding in bindings)
          binding.watch().firstWhere((rows) => rows.length == 2),
      ];
      await Future.wait([
        for (final table in tables)
          setup.query("INSERT INTO $table (id, value) VALUES (2, 'live')"),
      ]);
      final rowsByTable = await _withSyncErrors(kalam, Future.wait(completed))
          .timeout(
            const Duration(seconds: 30),
            onTimeout: () => throw StateError(
              'multi-table sync stalled: state=${kalam.syncState}, '
              'phase=${kalam.syncState.phase}, error=${kalam.syncState.error}, '
              'applied=$appliedChanges, '
              'batches=${transport.deliveredBatchSizes}, '
              'acks=${transport.acknowledged}',
            ),
          );

      expect(rowsByTable.every((rows) => rows.length == 2), isTrue);
      expect(kalam.syncState.phase, KalamSyncPhase.live);
      for (final table in tables) {
        final id = '$table/all';
        expect(transport.acknowledged[id], isNotEmpty);
        expect(
          await kalam.store.readCheckpoint(
            accountKey: kalam.identity.accountKey,
            subscriptionId: id,
          ),
          isNotNull,
        );
      }
    },
    skip: skipIfNoIntegration,
    timeout: const Timeout(Duration(seconds: 60)),
  );
}

Future<T> _withSyncErrors<T>(Kalam kalam, Future<T> result) {
  final failure = kalam.syncStates
      .firstWhere((state) => state.phase == KalamSyncPhase.error)
      .then<T>((state) => throw StateError('sync failed: ${state.error}'));
  return Future.any([result, failure]);
}
