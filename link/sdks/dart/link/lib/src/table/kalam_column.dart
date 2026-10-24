part of 'kalam_remote_table.dart';

final class KalamColumn<T> {
  const KalamColumn(this.name);

  final String name;

  KalamPredicate eq(T value) => value == null ? isNull() : _compare('=', value);
  KalamPredicate neq(T value) =>
      value == null ? isNotNull() : _compare('<>', value);
  KalamPredicate gt(T value) => _compare('>', value);
  KalamPredicate gte(T value) => _compare('>=', value);
  KalamPredicate lt(T value) => _compare('<', value);
  KalamPredicate lte(T value) => _compare('<=', value);

  KalamPredicate isNull() =>
      KalamPredicate._((_) => '${quoteKalamIdent(name)} IS NULL');

  KalamPredicate isNotNull() =>
      KalamPredicate._((_) => '${quoteKalamIdent(name)} IS NOT NULL');

  KalamPredicate inList(List<T> values) {
    if (values.isEmpty || values.length > _maxInList) {
      throw ArgumentError.value(
        values.length,
        'values',
        'inList accepts 1 to $_maxInList values',
      );
    }
    final captured = List<Object?>.unmodifiable(values);
    return KalamPredicate._((params) {
      final placeholders = [
        for (final value in captured)
          () {
            params.add(value);
            return '\$${params.length}';
          }(),
      ];
      return '${quoteKalamIdent(name)} IN (${placeholders.join(', ')})';
    });
  }

  KalamOrder asc() => KalamOrder([KalamOrderKey(name, descending: false)]);
  KalamOrder desc() => KalamOrder([KalamOrderKey(name, descending: true)]);

  KalamPredicate _compare(String operator, Object? value) {
    if (value == null) {
      throw ArgumentError('Ordering comparisons require a non-null value');
    }
    return KalamPredicate._((params) {
      params.add(value);
      return '${quoteKalamIdent(name)} $operator \$${params.length}';
    });
  }
}
