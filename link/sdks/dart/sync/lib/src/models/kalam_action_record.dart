import 'kalam_action_status.dart';

/// Public read model for one action stored in the durable outbox.
final class KalamActionRecord {
  const KalamActionRecord({
    required this.id,
    required this.accountKey,
    required this.actionKey,
    required this.version,
    required this.kind,
    required this.payloadJson,
    required this.status,
    required this.attemptCount,
    required this.createdAt,
    required this.updatedAt,
    this.orderingKey,
    this.rowTableId,
    this.rowKey,
    this.nextAttemptAt,
    this.lastError,
  });

  final String id;
  final String accountKey;
  final String actionKey;
  final int version;
  final String kind;
  final String payloadJson;
  final KalamActionStatus status;
  final String? orderingKey;
  final String? rowTableId;
  final String? rowKey;
  final int attemptCount;
  final DateTime? nextAttemptAt;
  final String? lastError;
  final DateTime createdAt;
  final DateTime updatedAt;

  String get idempotencyKey => id;

  bool get isTerminal => switch (status) {
    KalamActionStatus.succeeded ||
    KalamActionStatus.failed ||
    KalamActionStatus.cancelled => true,
    KalamActionStatus.queued ||
    KalamActionStatus.running ||
    KalamActionStatus.retryScheduled => false,
  };
}
