import 'dart:async';
import 'dart:convert';

import '../models/kalam_action_draft.dart';
import '../models/kalam_action_record.dart';
import '../models/kalam_change.dart';
import '../models/kalam_optimistic_row.dart';
import '../models/kalam_optimistic_mutation.dart';
import '../models/kalam_row_sync_state.dart';
import '../models/kalam_sync_mode.dart';
import '../models/kalam_synced_row.dart';
import '../store/kalam_sync_store.dart';
import '../sync/kalam_event_consumer.dart';
import '../transport/kalam_remote_change.dart';
import 'kalam_replica_overlay.dart';
import 'kalam_table_spec.dart';

typedef KalamRowWatch<T> = Stream<List<T>> Function();
typedef KalamRowUpsert<T> = Future<void> Function(T row);
typedef KalamRowDelete = Future<void> Function(String rowKey);

/// Binds generated Drift row types to Kalam's durable table policy.
final class KalamTableBinding<T> {
  KalamTableBinding({
    required this.spec,
    required this.accountKey,
    required this.store,
    KalamRowWatch<T>? watchLocal,
    KalamRowUpsert<T>? upsertLocal,
    KalamRowDelete? deleteLocal,
  }) : _watchLocal =
           watchLocal ??
           (() => store
               .watchCachedRows(accountKey: accountKey, tableId: spec.tableId)
               .map(
                 (rows) => [
                   for (final row in rows)
                     spec.decode(
                       Map<String, Object?>.from(
                         jsonDecode(row.valuesJson) as Map,
                       ),
                     ),
                 ],
               )),
       _upsertLocal =
           upsertLocal ??
           ((row) => store.upsertCachedRow(
             accountKey: accountKey,
             tableId: spec.tableId,
             rowKey: spec.keyOf(row),
             valuesJson: jsonEncode(spec.encode(row)),
           )),
       _deleteLocal =
           deleteLocal ??
           ((rowKey) => store.deleteCachedRow(
             accountKey: accountKey,
             tableId: spec.tableId,
             rowKey: rowKey,
           )),
       _overlay = KalamReplicaOverlay(spec);

  static const dmlActionKey = 'kalam.dml';

  final KalamTableSpec<T> spec;
  final String accountKey;
  final KalamSyncStore store;
  final KalamRowWatch<T> _watchLocal;
  final KalamRowUpsert<T> _upsertLocal;
  final KalamRowDelete _deleteLocal;
  final KalamReplicaOverlay<T> _overlay;

  Stream<List<T>> watch() =>
      watchWithSyncState().map((rows) => rows.map((row) => row.value).toList());

  Stream<List<KalamSyncedRow<T>>> watchWithSyncState() {
    return _combine(
      _watchLocal(),
      store.watchRowOverlays(accountKey: accountKey, tableId: spec.tableId),
      _overlay.merge,
    );
  }

  /// Creates a durable consumer that starts only when a widget/service uses it.
  KalamEventConsumer consumer({
    required String sql,
    int? batchSize,
    List<Object?>? params,
  }) {
    return KalamEventConsumer(
      id: spec.subscriptionId,
      sql: sql,
      apply: applyRemoteChange,
      batchSize: batchSize,
      params: params,
    );
  }

  Future<KalamActionRecord> insert(T row, {required String actionId}) {
    return _write('insert', row, actionId);
  }

  KalamOptimisticMutation optimisticInsert(T row) {
    final rowKey = spec.keyOf(row);
    final valuesJson = jsonEncode(spec.encode(row));
    return KalamOptimisticMutation(
      row: KalamOptimisticRow(
        tableId: spec.tableId,
        rowKey: rowKey,
        phase: KalamRowSyncPhase.pending,
        pendingValuesJson: valuesJson,
      ),
      apply: () => _upsertLocal(row),
    );
  }

  KalamOptimisticMutation optimisticDelete(String rowKey) {
    return KalamOptimisticMutation(
      row: KalamOptimisticRow(
        tableId: spec.tableId,
        rowKey: rowKey,
        phase: KalamRowSyncPhase.pendingDelete,
        tombstone: true,
      ),
      apply: () => _deleteLocal(rowKey),
    );
  }

  Future<KalamActionRecord> update(T row, {required String actionId}) {
    return _write('update', row, actionId);
  }

  Future<KalamActionRecord> delete(String rowKey, {required String actionId}) {
    _requireBidirectional();
    final payload = <String, Object?>{
      'operation': 'delete',
      'table': spec.tableId,
      'keyColumn': spec.keyColumn,
      'rowKey': rowKey,
    };
    return store.enqueue(
      KalamActionDraft(
        id: actionId,
        accountKey: accountKey,
        actionKey: dmlActionKey,
        kind: 'dml',
        payloadJson: jsonEncode(payload),
        orderingKey: '${spec.tableId}/$rowKey',
      ),
      optimisticRow: KalamOptimisticRow(
        tableId: spec.tableId,
        rowKey: rowKey,
        phase: KalamRowSyncPhase.pendingDelete,
        tombstone: true,
      ),
      applyOptimistic: () => _deleteLocal(rowKey),
    );
  }

  /// Applies a backend change and its resume checkpoint as one store operation.
  Future<bool> applyServerChange(KalamChange<T> change) {
    return store.applyAndCheckpoint(
      accountKey: accountKey,
      subscriptionId: spec.subscriptionId,
      seq: change.seq,
      apply: () => _applyChange(change),
    );
  }

  /// Applies one decoded transport row inside the coordinator's transaction.
  Future<void> applyRemoteChange(KalamRemoteChange remote) {
    final row = spec.decode(remote.row);
    return _applyChange(
      KalamChange(
        kind: remote.kind,
        rowKey: spec.keyOf(row),
        seq: remote.seq,
        row: remote.kind == KalamChangeKind.delete ? null : row,
      ),
    );
  }

  Future<KalamActionRecord> _write(String operation, T row, String actionId) {
    _requireBidirectional();
    final rowKey = spec.keyOf(row);
    final values = spec.encode(row);
    return store.enqueue(
      KalamActionDraft(
        id: actionId,
        accountKey: accountKey,
        actionKey: dmlActionKey,
        kind: 'dml',
        payloadJson: jsonEncode({
          'operation': operation,
          'table': spec.tableId,
          'keyColumn': spec.keyColumn,
          'rowKey': rowKey,
          'values': values,
        }),
        orderingKey: '${spec.tableId}/$rowKey',
      ),
      optimisticRow: KalamOptimisticRow(
        tableId: spec.tableId,
        rowKey: rowKey,
        phase: KalamRowSyncPhase.pending,
        pendingValuesJson: jsonEncode(values),
      ),
      applyOptimistic: () => _upsertLocal(row),
    );
  }

  Future<void> _applyChange(KalamChange<T> change) async {
    if (change.kind == KalamChangeKind.delete) {
      await _deleteLocal(change.rowKey);
      await store.removeRowState(
        accountKey: accountKey,
        tableId: spec.tableId,
        rowKey: change.rowKey,
      );
      return;
    }
    final row = change.row;
    if (row == null) throw StateError('${change.kind.name} requires a row.');
    await _upsertLocal(row);
    await store.markRowSynced(
      accountKey: accountKey,
      tableId: spec.tableId,
      rowKey: change.rowKey,
      seq: change.seq,
    );
  }

  void _requireBidirectional() {
    if (spec.mode == KalamSyncMode.replicaOnly) {
      throw StateError(
        '${spec.tableId} is replicaOnly. Enqueue a registered custom action '
        'with an optimistic row instead.',
      );
    }
  }
}

Stream<R> _combine<A, B, R>(
  Stream<A> first,
  Stream<B> second,
  R Function(A first, B second) combine,
) {
  late StreamController<R> controller;
  StreamSubscription<A>? firstSubscription;
  StreamSubscription<B>? secondSubscription;
  A? latestFirst;
  B? latestSecond;
  var hasFirst = false;
  var hasSecond = false;

  void emit() {
    if (hasFirst && hasSecond) {
      controller.add(combine(latestFirst as A, latestSecond as B));
    }
  }

  controller = StreamController<R>(
    onListen: () {
      firstSubscription = first.listen((value) {
        latestFirst = value;
        hasFirst = true;
        emit();
      }, onError: controller.addError);
      secondSubscription = second.listen((value) {
        latestSecond = value;
        hasSecond = true;
        emit();
      }, onError: controller.addError);
    },
    onCancel: () async {
      await firstSubscription?.cancel();
      await secondSubscription?.cancel();
    },
  );
  return controller.stream;
}
