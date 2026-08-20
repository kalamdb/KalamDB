import '../actions/kalam_action_runner.dart';
import '../models/kalam_action_record.dart';
import '../models/kalam_optimistic_mutation.dart';
import '../models/kalam_optimistic_row.dart';

/// Side-effect handle available while a durable consumer applies one event.
///
/// Enqueue work here so it commits with the checkpoint. Do not perform HTTP
/// or other I/O inside the apply transaction.
final class KalamConsumeContext {
  KalamConsumeContext({required KalamActionRunner actions}) : _actions = actions;

  final KalamActionRunner _actions;

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
