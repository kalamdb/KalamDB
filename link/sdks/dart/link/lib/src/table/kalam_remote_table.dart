import '../cell_value.dart';
import '../kalam_client.dart';
import '../models.dart';
import '../sql_params.dart';

part 'kalam_table_client.dart';
part 'kalam_mutation_result.dart';
part 'kalam_compiled_query.dart';
part 'kalam_table_query.dart';
part 'kalam_columns.dart';
part 'kalam_column.dart';
part 'kalam_order.dart';
part 'kalam_order_key.dart';
part 'kalam_predicate.dart';

const _maxInList = 100;

/// Quotes one SQL identifier. The value is a name, never a literal.
String quoteKalamIdent(String ident) {
  if (ident.isEmpty || ident.contains('\u0000')) {
    throw ArgumentError.value(ident, 'ident', 'SQL identifier is empty');
  }
  return '"${ident.replaceAll('"', '""')}"';
}

/// Quotes `schema.table` or a single table name.
String quoteKalamTable(String tableId) {
  final parts = tableId.split('.');
  if (parts.isEmpty || parts.any((part) => part.isEmpty)) {
    throw ArgumentError.value(tableId, 'tableId', 'SQL table id is empty');
  }
  return parts.map(quoteKalamIdent).join('.');
}

/// One remote table addressed by the existing [KalamClient] query and live APIs.
final class KalamRemoteTable<Row> {
  const KalamRemoteTable({
    required this.tableId,
    required this.keyColumn,
    required this.decode,
  });

  final String tableId;
  final String keyColumn;
  final Row Function(Map<String, Object?> json) decode;

  KalamTableClient<Row> bind(KalamClient client) {
    return KalamTableClient(client, this);
  }

  /// Builds SQL without a connection. Execution methods require [bind].
  KalamTableClient<Row> plan() => KalamTableClient.plan(this);

  Map<String, Object?> plainRow(Map<String, KalamCellValue> row) {
    return {for (final entry in row.entries) entry.key: entry.value.toJson()};
  }
}
