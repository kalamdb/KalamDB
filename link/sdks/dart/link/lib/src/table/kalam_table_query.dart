part of 'kalam_remote_table.dart';

/// Immutable filter over one remote table.
final class KalamTableQuery<Row> {
  const KalamTableQuery._(
    KalamTableClient<Row> client, {
    List<KalamPredicate> filters = const [],
    List<KalamOrderKey> orders = const [],
    int? limit,
  })  : _client = client,
        _filters = filters,
        _orders = orders,
        _limit = limit;

  final KalamTableClient<Row> _client;
  final List<KalamPredicate> _filters;
  final List<KalamOrderKey> _orders;
  final int? _limit;

  KalamTableQuery<Row> where(
    KalamPredicate Function(KalamColumns columns) build,
  ) {
    return filter(build(const KalamColumns()));
  }

  /// Composes generated typed columns without string column names.
  KalamTableQuery<Row> filter(KalamPredicate predicate) {
    return KalamTableQuery._(
      _client,
      filters: [..._filters, predicate],
      orders: _orders,
      limit: _limit,
    );
  }

  KalamTableQuery<Row> orderBy(
      KalamOrder Function(KalamColumns columns) build) {
    return KalamTableQuery._(
      _client,
      filters: _filters,
      orders: [..._orders, ...build(const KalamColumns()).keys],
      limit: _limit,
    );
  }

  KalamTableQuery<Row> limit(int count) {
    if (count <= 0) {
      throw ArgumentError.value(count, 'count', 'limit must be positive');
    }
    return KalamTableQuery._(
      _client,
      filters: _filters,
      orders: _orders,
      limit: count,
    );
  }

  KalamCompiledQuery compile({bool live = false}) {
    final params = <Object?>[];
    final where = _filters.isEmpty
        ? ''
        : ' WHERE ${_filters.map((filter) => '(${filter.render(params)})').join(' AND ')}';
    final table = quoteKalamTable(_client.table.tableId);
    if (live) {
      return KalamCompiledQuery('SELECT * FROM $table$where', params);
    }
    final order = _orders.isEmpty
        ? ''
        : ' ORDER BY ${_orders.map((key) => '${quoteKalamIdent(key.column)} ${key.descending ? 'DESC' : 'ASC'}').join(', ')}';
    final limit = _limit == null ? '' : ' LIMIT $_limit';
    return KalamCompiledQuery('SELECT * FROM $table$where$order$limit', params);
  }

  Future<List<Row>> get() async {
    final compiled = compile();
    final rows = await _client._rows(compiled);
    return [
      for (final row in rows) _client.table.decode(_client.table.plainRow(row)),
    ];
  }

  Future<Row> getOne() async {
    final rows = await limit(2).get();
    if (rows.length != 1) {
      throw StateError('getOne returned ${rows.length} rows');
    }
    return rows.first;
  }

  Future<Row?> getOptional() async {
    final rows = await limit(2).get();
    if (rows.isEmpty) return null;
    if (rows.length > 1) {
      throw StateError('getOptional returned ${rows.length} rows');
    }
    return rows.first;
  }

  /// Orders the complete matching live set before applying the display limit.
  /// Narrow the filter to bound memory; a display limit does not bound storage.
  Stream<List<Row>> live() {
    final compiled = compile(live: true);
    final sql = bindSqlParams(compiled.sql, compiled.params);
    return _client.client
        .live<Map<String, Object?>>(
      sql,
      keyColumns: [_client.table.keyColumn],
      mapRow: _client.table.plainRow,
    )
        .map((rows) {
      final ordered = [...rows];
      if (_orders.isNotEmpty) {
        ordered.sort((left, right) {
          for (final key in _orders) {
            final result =
                _compareLiveValues(left[key.column], right[key.column]);
            if (result != 0) return key.descending ? -result : result;
          }
          return _compareLiveValues(
              left[_client.table.keyColumn], right[_client.table.keyColumn]);
        });
      }
      final visible = _limit == null ? ordered : ordered.take(_limit);
      return visible.map(_client.table.decode).toList(growable: false);
    });
  }
}

int _compareLiveValues(Object? left, Object? right) {
  if (left == right) return 0;
  if (left == null) return 1;
  if (right == null) return -1;
  if (left is num && right is num) return left.compareTo(right);
  if (left is BigInt && right is BigInt) return left.compareTo(right);
  if (left is DateTime && right is DateTime) return left.compareTo(right);
  if (left is String && right is String) return left.compareTo(right);
  if (left is bool && right is bool) return left ? 1 : -1;
  throw StateError('Live ordering requires comparable scalar column values');
}
