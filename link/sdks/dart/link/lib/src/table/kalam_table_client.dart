part of 'kalam_remote_table.dart';

/// Bound table manager. Maps omit absent fields; a present null is SQL NULL.
final class KalamTableClient<Row> {
  const KalamTableClient(KalamClient client, this.table) : _client = client;

  const KalamTableClient.plan(this.table) : _client = null;

  final KalamClient? _client;
  final KalamRemoteTable<Row> table;

  KalamClient get client {
    final current = _client;
    if (current == null) {
      throw StateError('Bind the table to a KalamClient before executing');
    }
    return current;
  }

  KalamTableQuery<Row> query() => KalamTableQuery._(this);

  Future<Row?> get(Object? key) async {
    final compiled = _keyed('SELECT *', key);
    final rows = await _rows(compiled);
    if (rows.isEmpty) return null;
    if (rows.length > 1) {
      throw StateError('get returned ${rows.length} rows');
    }
    return table.decode(table.plainRow(rows.first));
  }

  Future<KalamMutationResult> insert(Map<String, Object?> values) {
    if (values.isEmpty) {
      throw ArgumentError('insert requires at least one column');
    }
    final params = <Object?>[];
    final columns = <String>[];
    final placeholders = <String>[];
    for (final entry in values.entries) {
      params.add(entry.value);
      columns.add(quoteKalamIdent(entry.key));
      placeholders.add('\$${params.length}');
    }
    final sql = 'INSERT INTO ${quoteKalamTable(table.tableId)} '
        '(${columns.join(', ')}) VALUES (${placeholders.join(', ')})';
    return _mutate(KalamCompiledQuery(sql, params));
  }

  Future<KalamMutationResult> update(Object? key, Map<String, Object?> patch) {
    if (patch.isEmpty) {
      throw ArgumentError('update requires at least one column');
    }
    final params = <Object?>[];
    final assignments = <String>[];
    for (final entry in patch.entries) {
      params.add(entry.value);
      assignments.add(
        '${quoteKalamIdent(entry.key)} = \$${params.length}',
      );
    }
    params.add(key);
    final sql =
        'UPDATE ${quoteKalamTable(table.tableId)} SET ${assignments.join(', ')} '
        'WHERE ${quoteKalamIdent(table.keyColumn)} = \$${params.length}';
    return _mutate(KalamCompiledQuery(sql, params));
  }

  Future<KalamMutationResult> delete(Object? key) {
    return _mutate(_keyed('DELETE', key));
  }

  KalamCompiledQuery _keyed(String verb, Object? key) {
    final sql = '$verb FROM ${quoteKalamTable(table.tableId)} '
        'WHERE ${quoteKalamIdent(table.keyColumn)} = \$1';
    return KalamCompiledQuery(sql, [key]);
  }

  Future<List<Map<String, KalamCellValue>>> _rows(
    KalamCompiledQuery compiled,
  ) async {
    final response = await client.query(compiled.sql, params: compiled.params);
    if (!response.success) {
      throw StateError(response.error?.message ?? 'query failed');
    }
    return response.rows;
  }

  Future<KalamMutationResult> _mutate(KalamCompiledQuery compiled) async {
    final response = await client.query(compiled.sql, params: compiled.params);
    if (!response.success) {
      throw StateError(response.error?.message ?? 'mutation failed');
    }
    final count =
        response.results.isEmpty ? 0 : response.results.first.rowCount;
    return KalamMutationResult(rowCount: count, response: response);
  }
}
