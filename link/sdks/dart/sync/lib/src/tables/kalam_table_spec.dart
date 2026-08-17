import '../models/kalam_sync_mode.dart';

/// Generated metadata and codecs for one Drift-owned application table.
final class KalamTableSpec<T> {
  const KalamTableSpec({
    required this.tableId,
    required this.keyColumn,
    required this.mode,
    required this.keyOf,
    required this.encode,
    required this.decode,
    String? subscriptionId,
  }) : subscriptionId = subscriptionId ?? tableId;

  final String tableId;
  final String keyColumn;
  final String subscriptionId;
  final KalamSyncMode mode;
  final String Function(T row) keyOf;
  final Map<String, Object?> Function(T row) encode;
  final T Function(Map<String, Object?> json) decode;
}
