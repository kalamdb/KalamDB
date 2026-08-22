enum KalamDmlOperation { insert, update, delete }

/// One generic direct-DML action; Drift continues to own the row types.
final class KalamDmlPayload {
  const KalamDmlPayload({
    required this.operation,
    required this.tableId,
    required this.keyColumn,
    required this.rowKey,
    required this.values,
  });

  final KalamDmlOperation operation;
  final String tableId;
  final String keyColumn;
  final Object? rowKey;
  final Map<String, Object?> values;
}
