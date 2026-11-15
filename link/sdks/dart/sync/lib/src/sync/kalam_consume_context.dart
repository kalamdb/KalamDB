import 'dart:convert';

import 'package:crypto/crypto.dart' show sha256;
import 'package:kalam_link/kalam_link.dart' show VersionId;

import '../actions/kalam_action_runner.dart';
import '../models/kalam_action_record.dart';
import '../models/kalam_optimistic_mutation.dart';
import '../models/kalam_optimistic_row.dart';

/// Side-effect handle available while a durable consumer applies one event.
///
/// Enqueue work here so it commits with the checkpoint. Do not perform HTTP
/// or other I/O inside the apply transaction.
final class KalamConsumeContext {
  KalamConsumeContext({required KalamActionRunner actions})
    : _actions = actions;

  final KalamActionRunner _actions;

  /// Enqueues follow-up work for a durable row event.
  ///
  /// The SDK derives a stable action ID from the account, consumer, action,
  /// row key and version. Replaying the same event deduplicates the action;
  /// changing its payload still raises an error instead of silently dropping
  /// or duplicating different work. Existing explicit IDs are left untouched.
  Future<KalamActionRecord> enqueueForEvent({
    required String actionKey,
    required String consumerId,
    required String rowKey,
    required VersionId version,
    required Object? payload,
    String? orderingKey,
    KalamOptimisticRow? optimisticRow,
    Future<void> Function()? applyOptimistic,
    KalamOptimisticMutation? optimistic,
  }) {
    if (consumerId.isEmpty || rowKey.isEmpty || actionKey.isEmpty) {
      return Future.error(
        ArgumentError('Event action identity must not be empty'),
      );
    }
    final identity = jsonEncode([
      _actions.accountKey,
      consumerId,
      actionKey,
      rowKey,
      version.value.toString(),
    ]);
    final id = 'kalam:event:v1:${sha256.convert(utf8.encode(identity))}';
    return enqueue(
      actionKey: actionKey,
      actionId: id,
      payload: payload,
      orderingKey: orderingKey,
      optimisticRow: optimisticRow,
      applyOptimistic: applyOptimistic,
      optimistic: optimistic,
    );
  }

  Future<KalamActionRecord> enqueue({
    required String actionKey,
    required String actionId,
    required Object? payload,
    String? orderingKey,
    KalamOptimisticRow? optimisticRow,
    Future<void> Function()? applyOptimistic,
    KalamOptimisticMutation? optimistic,
  }) {
    return _actions.enqueue(
      actionKey: actionKey,
      actionId: actionId,
      payload: payload,
      orderingKey: orderingKey,
      optimisticRow: optimisticRow,
      applyOptimistic: applyOptimistic,
      optimistic: optimistic,
    );
  }
}
