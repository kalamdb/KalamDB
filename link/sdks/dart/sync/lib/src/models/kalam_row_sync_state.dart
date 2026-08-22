/// Local synchronization phase for one row.
enum KalamRowSyncPhase {
  pending,
  sending,
  awaitingServerEcho,
  pendingDelete,
  synced,
  failed,
}

/// SDK-owned metadata associated with a server-shaped row.
final class KalamRowSyncState {
  const KalamRowSyncState({
    required this.phase,
    this.actionId,
    this.attemptCount = 0,
    this.nextRetryAt,
    this.errorCode,
    this.errorMessage,
    this.lastServerSeq,
  });

  const KalamRowSyncState.synced({String? lastServerSeq})
    : this(phase: KalamRowSyncPhase.synced, lastServerSeq: lastServerSeq);

  final KalamRowSyncPhase phase;
  final String? actionId;
  final int attemptCount;
  final DateTime? nextRetryAt;
  final String? errorCode;
  final String? errorMessage;
  final String? lastServerSeq;

  bool get isPending => switch (phase) {
    KalamRowSyncPhase.pending ||
    KalamRowSyncPhase.sending ||
    KalamRowSyncPhase.awaitingServerEcho ||
    KalamRowSyncPhase.pendingDelete => true,
    KalamRowSyncPhase.synced || KalamRowSyncPhase.failed => false,
  };

  bool get isFailed => phase == KalamRowSyncPhase.failed;
  bool get isSynced => phase == KalamRowSyncPhase.synced;

  @override
  bool operator ==(Object other) {
    return other is KalamRowSyncState &&
        other.phase == phase &&
        other.actionId == actionId &&
        other.attemptCount == attemptCount &&
        other.nextRetryAt == nextRetryAt &&
        other.errorCode == errorCode &&
        other.errorMessage == errorMessage &&
        other.lastServerSeq == lastServerSeq;
  }

  @override
  int get hashCode => Object.hash(
    phase,
    actionId,
    attemptCount,
    nextRetryAt,
    errorCode,
    errorMessage,
    lastServerSeq,
  );
}
