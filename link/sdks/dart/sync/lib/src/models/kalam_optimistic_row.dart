import 'kalam_row_sync_state.dart';

/// Pending row metadata committed with a durable action.
final class KalamOptimisticRow {
  const KalamOptimisticRow({
    required this.tableId,
    required this.rowKey,
    required this.phase,
    this.pendingValuesJson,
    this.tombstone = false,
  });

  final String tableId;
  final String rowKey;
  final KalamRowSyncPhase phase;
  final String? pendingValuesJson;
  final bool tombstone;
}
