import 'package:flutter_test/flutter_test.dart';
import 'package:kalam_link/kalam_link.dart';

void main() {
  test('leaves SQL unchanged when no params are supplied', () {
    const sql = 'SELECT * FROM public.messages WHERE conversation_id = \$1';
    expect(bindSqlParams(sql, null), sql);
    expect(bindSqlParams(sql, const []), sql);
  });

  test('binds numbered placeholders from highest to lowest', () {
    expect(
      bindSqlParams(
        'SELECT * FROM t WHERE a = \$1 AND b = \$10 AND c = \$2',
        ['first', 2, 3, 4, 5, 6, 7, 8, 9, 10],
      ),
      "SELECT * FROM t WHERE a = 'first' AND b = 10 AND c = 2",
    );
  });

  test('quotes strings, booleans, numbers, and null', () {
    expect(
      bindSqlParams(
        'SELECT * FROM t WHERE id = \$1 AND ok = \$2 AND n = \$3 AND missing = \$4',
        ["O'Brien", true, 42, null],
      ),
      "SELECT * FROM t WHERE id = 'O''Brien' AND ok = TRUE AND n = 42 AND missing = NULL",
    );
  });
}
