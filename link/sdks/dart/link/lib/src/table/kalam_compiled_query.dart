part of 'kalam_remote_table.dart';

final class KalamCompiledQuery {
  const KalamCompiledQuery(this.sql, this.params);

  final String sql;
  final List<Object?> params;
}
