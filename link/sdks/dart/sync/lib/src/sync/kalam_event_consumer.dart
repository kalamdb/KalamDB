import 'dart:async';

import '../transport/kalam_remote_change.dart';
import 'kalam_consume_context.dart';

typedef KalamChangeApplier = FutureOr<void> Function(KalamRemoteChange change);
typedef KalamChangeConsumer =
    FutureOr<void> Function(
      KalamRemoteChange change,
      KalamConsumeContext context,
    );

/// A widget- or service-scoped durable subscription.
final class KalamEventConsumer {
  const KalamEventConsumer({
    required this.id,
    required this.sql,
    this.apply,
    this.onEvent,
    this.batchSize,
    this.params,
  }) : assert(
         apply != null || onEvent != null,
         'KalamEventConsumer requires apply or onEvent.',
       );

  /// Durable side-effect consumer. [onEvent] may enqueue follow-up actions
  /// in the same SQLite transaction as the checkpoint. Do not perform HTTP.
  factory KalamEventConsumer.consume({
    required String id,
    required String sql,
    required KalamChangeConsumer onEvent,
    int? batchSize,
    List<Object?>? params,
  }) {
    return KalamEventConsumer(
      id: id,
      sql: sql,
      onEvent: onEvent,
      batchSize: batchSize,
      params: params,
    );
  }

  final String id;
  final String sql;
  final KalamChangeApplier? apply;
  final KalamChangeConsumer? onEvent;
  final int? batchSize;
  final List<Object?>? params;

  FutureOr<void> handle(
    KalamRemoteChange change,
    KalamConsumeContext context,
  ) {
    final consume = onEvent;
    if (consume != null) return consume(change, context);
    return apply!(change);
  }
}
