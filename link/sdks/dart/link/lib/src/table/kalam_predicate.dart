part of 'kalam_remote_table.dart';

final class KalamPredicate {
  const KalamPredicate._(this._render);

  final String Function(List<Object?> params) _render;

  String render(List<Object?> params) => _render(params);

  static KalamPredicate and(List<KalamPredicate> items) => _group('AND', items);
  static KalamPredicate or(List<KalamPredicate> items) => _group('OR', items);

  static KalamPredicate _group(String operator, List<KalamPredicate> items) {
    if (items.isEmpty) {
      throw ArgumentError('$operator requires a predicate');
    }
    final captured = List<KalamPredicate>.unmodifiable(items);
    return KalamPredicate._((params) {
      final rendered = captured
          .map((item) => '(${item.render(params)})')
          .join(' $operator ');
      return rendered;
    });
  }
}
