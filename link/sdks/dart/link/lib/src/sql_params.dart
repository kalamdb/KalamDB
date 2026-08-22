/// Binds `$1`, `$2`, ... placeholders into a SQL string.
///
/// Live subscriptions currently send SQL through the native
/// [SubscriptionConfig] without a separate params field. Binding here keeps
/// `liveEvents` / `liveEventsWithAck` aligned with `query(params: ...)`.
String bindSqlParams(String sql, List<Object?>? params) {
  if (params == null || params.isEmpty) return sql;
  var bound = sql;
  for (var index = params.length; index >= 1; index--) {
    bound = bound.replaceAll('\$$index', sqlLiteral(params[index - 1]));
  }
  return bound;
}

String sqlLiteral(Object? value) {
  if (value == null) return 'NULL';
  if (value is bool) return value ? 'TRUE' : 'FALSE';
  if (value is num) return '$value';
  if (value is DateTime) {
    return "'${value.toUtc().toIso8601String().replaceAll("'", "''")}'";
  }
  final text = value.toString().replaceAll("'", "''");
  return "'$text'";
}
