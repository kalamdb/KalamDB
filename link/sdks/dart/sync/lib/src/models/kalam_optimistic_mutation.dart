import 'kalam_optimistic_row.dart';

/// One local row mutation committed atomically with a custom action.
final class KalamOptimisticMutation {
  const KalamOptimisticMutation({required this.row, required this.apply});

  final KalamOptimisticRow row;
  final Future<void> Function() apply;
}
