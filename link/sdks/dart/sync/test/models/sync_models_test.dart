import 'package:flutter_test/flutter_test.dart';
import 'package:kalam_sync/kalam_sync.dart';

void main() {
  group('KalamSyncState', () {
    test('starts offline with no pending work', () {
      const state = KalamSyncState();

      expect(state.phase, KalamSyncPhase.offline);
      expect(state.pendingActions, 0);
      expect(state.failedActions, 0);
      expect(state.hasPendingWork, isFalse);
    });

    test('copyWith preserves values and can clear an error', () {
      const original = KalamSyncState(
        phase: KalamSyncPhase.error,
        pendingActions: 2,
        failedActions: 1,
        error: 'network unavailable',
      );

      final updated = original.copyWith(
        phase: KalamSyncPhase.flushing,
        clearError: true,
      );

      expect(updated.phase, KalamSyncPhase.flushing);
      expect(updated.pendingActions, 2);
      expect(updated.failedActions, 1);
      expect(updated.error, isNull);
      expect(updated.hasPendingWork, isTrue);
    });
  });

  test('sync modes keep direct DML and replicas distinct', () {
    expect(KalamSyncMode.values, [
      KalamSyncMode.bidirectional,
      KalamSyncMode.replicaOnly,
    ]);
  });

  test('synced rows expose Drift values and local sync metadata', () {
    const sync = KalamRowSyncState(
      phase: KalamRowSyncPhase.pending,
      actionId: 'action-1',
      attemptCount: 1,
    );
    const row = KalamSyncedRow(value: 'hello', sync: sync);

    expect(row.value, 'hello');
    expect(row.sync.phase, KalamRowSyncPhase.pending);
    expect(row.isSynced, isFalse);
    expect(row, const KalamSyncedRow(value: 'hello', sync: sync));
  });

  test('row state distinguishes local sync from backend delivery state', () {
    const state = KalamRowSyncState(
      phase: KalamRowSyncPhase.awaitingServerEcho,
      actionId: 'action-2',
      lastServerSeq: '42',
    );

    expect(state.isPending, isTrue);
    expect(state.isFailed, isFalse);
    expect(state.lastServerSeq, '42');
  });

  test('action status includes durable retry and terminal states', () {
    expect(
      KalamActionStatus.values,
      containsAll([
        KalamActionStatus.queued,
        KalamActionStatus.running,
        KalamActionStatus.retryScheduled,
        KalamActionStatus.succeeded,
        KalamActionStatus.failed,
        KalamActionStatus.cancelled,
      ]),
    );
  });
}
