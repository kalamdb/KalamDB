import 'dart:async';

import '../transport/kalam_remote_change.dart';

typedef KalamChangeApplier = FutureOr<void> Function(KalamRemoteChange change);

/// A widget- or service-scoped durable subscription.
final class KalamEventConsumer {
  const KalamEventConsumer({
    required this.id,
    required this.sql,
    required this.apply,
    this.batchSize,
  });

  final String id;
  final String sql;
  final KalamChangeApplier apply;
  final int? batchSize;
}
