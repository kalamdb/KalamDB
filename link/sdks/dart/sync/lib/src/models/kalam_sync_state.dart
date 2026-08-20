/// High-level state of the local-first synchronization engine.
enum KalamSyncPhase { offline, catchingUp, flushing, live, paused, error }

/// Immutable state suitable for Flutter builders and state providers.
final class KalamSyncState {
  const KalamSyncState({
    this.phase = KalamSyncPhase.offline,
    this.pendingActions = 0,
    this.failedActions = 0,
    this.lastSuccessfulSync,
    this.error,
    this.errorCode,
  });

  final KalamSyncPhase phase;
  final int pendingActions;
  final int failedActions;
  final DateTime? lastSuccessfulSync;
  final String? error;
  final String? errorCode;

  bool get hasPendingWork => pendingActions > 0;

  bool get isAuthError =>
      errorCode == 'TOKEN_EXPIRED' || errorCode == 'UNAUTHORIZED';

  KalamSyncState copyWith({
    KalamSyncPhase? phase,
    int? pendingActions,
    int? failedActions,
    DateTime? lastSuccessfulSync,
    String? error,
    String? errorCode,
    bool clearError = false,
  }) {
    return KalamSyncState(
      phase: phase ?? this.phase,
      pendingActions: pendingActions ?? this.pendingActions,
      failedActions: failedActions ?? this.failedActions,
      lastSuccessfulSync: lastSuccessfulSync ?? this.lastSuccessfulSync,
      error: clearError ? null : error ?? this.error,
      errorCode: clearError ? null : errorCode ?? this.errorCode,
    );
  }

  @override
  bool operator ==(Object other) {
    return other is KalamSyncState &&
        other.phase == phase &&
        other.pendingActions == pendingActions &&
        other.failedActions == failedActions &&
        other.lastSuccessfulSync == lastSuccessfulSync &&
        other.error == error &&
        other.errorCode == errorCode;
  }

  @override
  int get hashCode => Object.hash(
    phase,
    pendingActions,
    failedActions,
    lastSuccessfulSync,
    error,
    errorCode,
  );
}
