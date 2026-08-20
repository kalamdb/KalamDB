import 'dart:async';

import 'package:drift/drift.dart';
import 'package:kalam_link/kalam_link.dart';

import '../database/kalam_sync_database.dart';
import '../models/kalam_action_draft.dart';
import '../models/kalam_action_record.dart';
import '../models/kalam_action_status.dart';
import '../models/kalam_checkpoint.dart';
import '../models/kalam_cached_row.dart';
import '../models/kalam_optimistic_row.dart';
import '../models/kalam_row_overlay.dart';
import '../models/kalam_row_sync_state.dart';

typedef KalamClock = DateTime Function();

/// Single-writer access to Kalam's private persistence tables.
final class KalamSyncStore {
  KalamSyncStore(this.database, {KalamClock? clock})
    : _clock = clock ?? DateTime.now;

  final KalamSyncDatabase database;
  final KalamClock _clock;

  Future<KalamActionRecord> enqueue(
    KalamActionDraft draft, {
    KalamOptimisticRow? optimisticRow,
    Future<void> Function()? applyOptimistic,
  }) {
    return database.transaction(() async {
      final now = _clock();
      final maxPosition = database.kalamActions.queuePosition.max();
      final positionQuery = database.selectOnly(database.kalamActions)
        ..addColumns([maxPosition]);
      final highestPosition = await positionQuery
          .map((row) => row.read(maxPosition))
          .getSingle();
      await database
          .into(database.kalamActions)
          .insert(
            KalamActionsCompanion.insert(
              id: draft.id,
              accountKey: draft.accountKey,
              actionKey: draft.actionKey,
              version: Value(draft.version),
              kind: Value(draft.kind),
              payloadJson: draft.payloadJson,
              status: KalamActionStatus.queued.name,
              orderingKey: Value(draft.orderingKey),
              rowTableId: Value(optimisticRow?.tableId),
              rowKey: Value(optimisticRow?.rowKey),
              queuePosition: Value((highestPosition ?? 0) + 1),
              createdAt: now,
              updatedAt: now,
            ),
          );

      if (optimisticRow != null) {
        await database
            .into(database.kalamRowStates)
            .insertOnConflictUpdate(
              KalamRowStatesCompanion.insert(
                accountKey: draft.accountKey,
                tableId: optimisticRow.tableId,
                rowKey: optimisticRow.rowKey,
                phase: optimisticRow.phase.name,
                actionId: Value(draft.id),
                pendingValuesJson: Value(optimisticRow.pendingValuesJson),
                tombstone: Value(optimisticRow.tombstone),
                updatedAt: now,
              ),
            );
      }

      await applyOptimistic?.call();

      final stored = await (database.select(
        database.kalamActions,
      )..where((row) => row.id.equals(draft.id))).getSingle();
      return _toActionRecord(stored);
    });
  }

  Stream<List<KalamActionRecord>> watchActions(String accountKey) {
    final query = database.select(database.kalamActions)
      ..where((row) => row.accountKey.equals(accountKey))
      ..orderBy([(row) => OrderingTerm.asc(row.queuePosition)]);
    return query.watch().map(
      (rows) => rows.map(_toActionRecord).toList(growable: false),
    );
  }

  Future<KalamActionRecord?> readAction(String id) async {
    final stored = await (database.select(
      database.kalamActions,
    )..where((row) => row.id.equals(id))).getSingleOrNull();
    return stored == null ? null : _toActionRecord(stored);
  }

  /// Returns and marks the oldest eligible action as running.
  ///
  /// Actions whose [orderingKey] is in [blockedOrderingKeys] are skipped so
  /// unrelated keys can flush concurrently. A null/empty ordering key shares
  /// one bucket (`''`) and stays FIFO with other unordered work.
  Future<KalamActionRecord?> claimNextAction(
    DateTime now, {
    String? accountKey,
    Set<String> blockedOrderingKeys = const {},
  }) {
    return database.transaction(() async {
      final query = database.select(database.kalamActions)
        ..where((row) {
          final eligible =
              row.status.equals(KalamActionStatus.queued.name) |
              (row.status.equals(KalamActionStatus.retryScheduled.name) &
                  row.nextAttemptAt.isSmallerOrEqualValue(now));
          return accountKey == null
              ? eligible
              : eligible & row.accountKey.equals(accountKey);
        })
        ..orderBy([(row) => OrderingTerm.asc(row.queuePosition)]);
      final candidates = await query.get();
      StoredAction? stored;
      for (final row in candidates) {
        final key = row.orderingKey ?? '';
        if (blockedOrderingKeys.contains(key)) continue;
        stored = row;
        break;
      }
      if (stored == null) return null;

      await (database.update(
        database.kalamActions,
      )..where((row) => row.id.equals(stored!.id))).write(
        KalamActionsCompanion(
          status: Value(KalamActionStatus.running.name),
          attemptCount: Value(stored.attemptCount + 1),
          nextAttemptAt: const Value(null),
          updatedAt: Value(now),
        ),
      );
      if (stored.rowTableId != null && stored.rowKey != null) {
        await _updateRowForAction(
          stored,
          phase: KalamRowSyncPhase.sending,
          attemptCount: stored.attemptCount + 1,
          now: now,
        );
      }
      return _toActionRecord(
        stored.copyWith(
          status: KalamActionStatus.running.name,
          attemptCount: stored.attemptCount + 1,
          nextAttemptAt: const Value(null),
          updatedAt: now,
        ),
      );
    });
  }

  /// Makes interrupted work eligible after a process/runtime restart.
  Future<int> recoverRunningActions(DateTime now, {String? accountKey}) {
    return database.transaction(() async {
      final query = database.select(database.kalamActions)
        ..where(
          (row) => accountKey == null
              ? row.status.equals(KalamActionStatus.running.name)
              : row.status.equals(KalamActionStatus.running.name) &
                    row.accountKey.equals(accountKey),
        );
      final interrupted = await query.get();
      if (interrupted.isEmpty) return 0;

      final update = database.update(database.kalamActions)
        ..where((row) => row.status.equals(KalamActionStatus.running.name));
      if (accountKey != null) {
        update.where((row) => row.accountKey.equals(accountKey));
      }
      final changed = await update.write(
        KalamActionsCompanion(
          status: Value(KalamActionStatus.queued.name),
          updatedAt: Value(now),
        ),
      );
      for (final action in interrupted) {
        await _updateRowForAction(
          action,
          phase: KalamRowSyncPhase.pending,
          now: now,
        );
      }
      return changed;
    });
  }

  Future<void> completeAction(String id, DateTime now) {
    return _setActionStatus(
      id,
      KalamActionStatus.succeeded,
      now,
      clearError: true,
      rowPhase: KalamRowSyncPhase.awaitingServerEcho,
    );
  }

  Future<void> failAction(String id, String error, DateTime now) {
    return _setActionStatus(
      id,
      KalamActionStatus.failed,
      now,
      error: error,
      rowPhase: KalamRowSyncPhase.failed,
    );
  }

  Future<void> scheduleRetry(String id, String error, DateTime retryAt) {
    return _setActionStatus(
      id,
      KalamActionStatus.retryScheduled,
      _clock(),
      error: error,
      retryAt: retryAt,
      rowPhase: KalamRowSyncPhase.pending,
    );
  }

  Stream<List<KalamRowOverlay>> watchRowOverlays({
    required String accountKey,
    required String tableId,
  }) {
    final query = database.select(database.kalamRowStates)
      ..where(
        (row) =>
            row.accountKey.equals(accountKey) & row.tableId.equals(tableId),
      );
    return query.watch().map(
      (rows) => rows.map(_toRowOverlay).toList(growable: false),
    );
  }

  Stream<List<KalamCachedRow>> watchCachedRows({
    required String accountKey,
    required String tableId,
  }) {
    final query = database.select(database.kalamCachedRows)
      ..where(
        (row) =>
            row.accountKey.equals(accountKey) & row.tableId.equals(tableId),
      )
      ..orderBy([(row) => OrderingTerm.asc(row.rowKey)]);
    return query.watch().map(
      (rows) => [
        for (final row in rows)
          KalamCachedRow(rowKey: row.rowKey, valuesJson: row.valuesJson),
      ],
    );
  }

  Future<void> upsertCachedRow({
    required String accountKey,
    required String tableId,
    required String rowKey,
    required String valuesJson,
  }) {
    return database
        .into(database.kalamCachedRows)
        .insertOnConflictUpdate(
          KalamCachedRowsCompanion.insert(
            accountKey: accountKey,
            tableId: tableId,
            rowKey: rowKey,
            valuesJson: valuesJson,
            updatedAt: _clock(),
          ),
        );
  }

  Future<void> deleteCachedRow({
    required String accountKey,
    required String tableId,
    required String rowKey,
  }) {
    return (database.delete(database.kalamCachedRows)..where(
          (row) =>
              row.accountKey.equals(accountKey) &
              row.tableId.equals(tableId) &
              row.rowKey.equals(rowKey),
        ))
        .go();
  }

  Future<void> markRowSynced({
    required String accountKey,
    required String tableId,
    required String rowKey,
    required SeqId seq,
  }) {
    return database.transaction(() async {
      final existing = await _readStoredRowState(
        accountKey: accountKey,
        tableId: tableId,
        rowKey: rowKey,
      );
      if (existing?.tombstone ?? false) {
        return;
      }
      await database
          .into(database.kalamRowStates)
          .insertOnConflictUpdate(
            KalamRowStatesCompanion.insert(
              accountKey: accountKey,
              tableId: tableId,
              rowKey: rowKey,
              phase: KalamRowSyncPhase.synced.name,
              actionId: const Value(null),
              attemptCount: const Value(0),
              nextRetryAt: const Value(null),
              errorCode: const Value(null),
              errorMessage: const Value(null),
              lastServerSeq: Value(seq.toString()),
              pendingValuesJson: const Value(null),
              tombstone: const Value(false),
              updatedAt: _clock(),
            ),
          );
      await _completeReconciledAction(existing?.actionId);
    });
  }

  Future<void> removeRowState({
    required String accountKey,
    required String tableId,
    required String rowKey,
  }) {
    return database.transaction(() async {
      final existing = await _readStoredRowState(
        accountKey: accountKey,
        tableId: tableId,
        rowKey: rowKey,
      );
      await (database.delete(database.kalamRowStates)..where(
            (row) =>
                row.accountKey.equals(accountKey) &
                row.tableId.equals(tableId) &
                row.rowKey.equals(rowKey),
          ))
          .go();
      await _completeReconciledAction(existing?.actionId);
    });
  }

  Future<String?> readCompletedStep(String actionId, String name) async {
    final step =
        await (database.select(database.kalamActionSteps)..where(
              (row) => row.actionId.equals(actionId) & row.name.equals(name),
            ))
            .getSingleOrNull();
    return step?.status == KalamActionStatus.succeeded.name
        ? step?.resultJson
        : null;
  }

  Future<void> completeStep(String actionId, String name, String resultJson) {
    return database
        .into(database.kalamActionSteps)
        .insertOnConflictUpdate(
          KalamActionStepsCompanion.insert(
            actionId: actionId,
            name: name,
            status: KalamActionStatus.succeeded.name,
            resultJson: Value(resultJson),
            updatedAt: _clock(),
          ),
        );
  }

  Future<void> failStep(String actionId, String name, String error) {
    return database
        .into(database.kalamActionSteps)
        .insertOnConflictUpdate(
          KalamActionStepsCompanion.insert(
            actionId: actionId,
            name: name,
            status: KalamActionStatus.failed.name,
            lastError: Value(error),
            updatedAt: _clock(),
          ),
        );
  }

  Future<KalamCheckpoint?> readCheckpoint({
    required String accountKey,
    required String subscriptionId,
  }) async {
    final stored =
        await (database.select(database.kalamCheckpoints)..where(
              (row) =>
                  row.accountKey.equals(accountKey) &
                  row.subscriptionId.equals(subscriptionId),
            ))
            .getSingleOrNull();
    if (stored == null) return null;
    return KalamCheckpoint(
      accountKey: stored.accountKey,
      subscriptionId: stored.subscriptionId,
      seq: SeqId.parse(stored.seq),
      updatedAt: stored.updatedAt.toUtc(),
    );
  }

  /// Runs [apply] and advances its checkpoint in one SQLite transaction.
  ///
  /// Returns `false` without invoking [apply] for duplicate or stale events.
  Future<bool> applyAndCheckpoint({
    required String accountKey,
    required String subscriptionId,
    required SeqId seq,
    required FutureOr<Object?> Function() apply,
  }) {
    return database.transaction(() async {
      final current = await readCheckpoint(
        accountKey: accountKey,
        subscriptionId: subscriptionId,
      );
      if (current != null && current.seq >= seq) return false;

      await apply();
      final now = _clock();
      await database
          .into(database.kalamCheckpoints)
          .insertOnConflictUpdate(
            KalamCheckpointsCompanion.insert(
              accountKey: accountKey,
              subscriptionId: subscriptionId,
              seq: seq.toString(),
              updatedAt: now,
            ),
          );
      return true;
    });
  }

  Future<void> deleteCheckpoint({
    required String accountKey,
    required String subscriptionId,
  }) {
    return (database.delete(database.kalamCheckpoints)..where(
          (row) =>
              row.accountKey.equals(accountKey) &
              row.subscriptionId.equals(subscriptionId),
        ))
        .go();
  }

  KalamActionRecord _toActionRecord(StoredAction stored) {
    return KalamActionRecord(
      id: stored.id,
      accountKey: stored.accountKey,
      actionKey: stored.actionKey,
      version: stored.version,
      kind: stored.kind,
      payloadJson: stored.payloadJson,
      status: KalamActionStatus.values.byName(stored.status),
      orderingKey: stored.orderingKey,
      rowTableId: stored.rowTableId,
      rowKey: stored.rowKey,
      attemptCount: stored.attemptCount,
      nextAttemptAt: stored.nextAttemptAt?.toUtc(),
      lastError: stored.lastError,
      createdAt: stored.createdAt.toUtc(),
      updatedAt: stored.updatedAt.toUtc(),
    );
  }

  Future<void> _setActionStatus(
    String id,
    KalamActionStatus status,
    DateTime now, {
    String? error,
    DateTime? retryAt,
    bool clearError = false,
    KalamRowSyncPhase? rowPhase,
  }) {
    return database.transaction(() async {
      final action = await (database.select(
        database.kalamActions,
      )..where((row) => row.id.equals(id))).getSingleOrNull();
      if (action == null) throw StateError('Unknown Kalam action "$id".');
      if (action.status == KalamActionStatus.succeeded.name ||
          action.status == KalamActionStatus.cancelled.name) {
        return;
      }

      await (database.update(
        database.kalamActions,
      )..where((row) => row.id.equals(id))).write(
        KalamActionsCompanion(
          status: Value(status.name),
          nextAttemptAt: Value(retryAt),
          lastError: clearError ? const Value(null) : Value(error),
          updatedAt: Value(now),
        ),
      );
      if (rowPhase != null) {
        await _updateRowForAction(
          action,
          phase: rowPhase,
          nextRetryAt: retryAt,
          errorMessage: error,
          now: now,
        );
      }
    });
  }

  Future<void> _updateRowForAction(
    StoredAction action, {
    required KalamRowSyncPhase phase,
    required DateTime now,
    int? attemptCount,
    DateTime? nextRetryAt,
    String? errorMessage,
  }) async {
    if (action.rowTableId == null || action.rowKey == null) return;
    await (database.update(database.kalamRowStates)..where(
          (row) =>
              row.accountKey.equals(action.accountKey) &
              row.tableId.equals(action.rowTableId!) &
              row.rowKey.equals(action.rowKey!),
        ))
        .write(
          KalamRowStatesCompanion(
            phase: Value(phase.name),
            attemptCount: attemptCount == null
                ? const Value.absent()
                : Value(attemptCount),
            nextRetryAt: Value(nextRetryAt),
            errorMessage: Value(errorMessage),
            updatedAt: Value(now),
          ),
        );
  }

  KalamRowOverlay _toRowOverlay(StoredRowState stored) {
    return KalamRowOverlay(
      rowKey: stored.rowKey,
      sync: KalamRowSyncState(
        phase: KalamRowSyncPhase.values.byName(stored.phase),
        actionId: stored.actionId,
        attemptCount: stored.attemptCount,
        nextRetryAt: stored.nextRetryAt?.toUtc(),
        errorCode: stored.errorCode,
        errorMessage: stored.errorMessage,
        lastServerSeq: stored.lastServerSeq,
      ),
      pendingValuesJson: stored.pendingValuesJson,
      tombstone: stored.tombstone,
    );
  }

  Future<StoredRowState?> _readStoredRowState({
    required String accountKey,
    required String tableId,
    required String rowKey,
  }) {
    return (database.select(database.kalamRowStates)..where(
          (row) =>
              row.accountKey.equals(accountKey) &
              row.tableId.equals(tableId) &
              row.rowKey.equals(rowKey),
        ))
        .getSingleOrNull();
  }

  Future<void> _completeReconciledAction(String? actionId) async {
    if (actionId == null) return;
    await (database.update(
      database.kalamActions,
    )..where((row) => row.id.equals(actionId))).write(
      KalamActionsCompanion(
        status: Value(KalamActionStatus.succeeded.name),
        nextAttemptAt: const Value(null),
        lastError: const Value(null),
        updatedAt: Value(_clock()),
      ),
    );
  }
}
