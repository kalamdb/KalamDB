import 'package:flutter_test/flutter_test.dart';
import 'package:kalam_link/kalam_link.dart';

void main() {
  const table = KalamRemoteTable<Map<String, Object?>>(
    tableId: 'public.messages',
    keyColumn: 'id',
    decode: _identity,
  );

  test('compiles a filtered ordered query with bound values', () {
    final compiled = table
        .plan()
        .query()
        .where((columns) => columns.col('conversation_id').eq("o'brien"))
        .where((columns) => columns.col('body').isNotNull())
        .orderBy((columns) => columns.col('created_at').desc())
        .limit(50)
        .compile();

    expect(
      compiled.sql,
      'SELECT * FROM "public"."messages" WHERE ("conversation_id" = \$1) '
      'AND ("body" IS NOT NULL) ORDER BY "created_at" DESC LIMIT 50',
    );
    expect(compiled.params, ["o'brien"]);
  });

  test('live sql keeps the filter and drops order and limit', () {
    final compiled = table
        .plan()
        .query()
        .where(
          (columns) => KalamPredicate.or([
            columns.col('id').inList(['a', 'b']),
            columns.col('sender_id').isNull(),
          ]),
        )
        .orderBy((columns) => columns.col('id').asc())
        .limit(10)
        .compile(live: true);

    expect(
      compiled.sql,
      'SELECT * FROM "public"."messages" WHERE (("id" IN (\$1, \$2)) '
      'OR ("sender_id" IS NULL))',
    );
    expect(compiled.params, ['a', 'b']);
  });

  test('rejects an empty in-list', () {
    expect(
      () => table.plan().query().where(
            (columns) => columns.col('id').inList(const []),
          ),
      throwsArgumentError,
    );
  });
}

Map<String, Object?> _identity(Map<String, Object?> json) => json;
