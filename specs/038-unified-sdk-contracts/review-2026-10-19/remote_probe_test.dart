import 'package:flutter_test/flutter_test.dart';
import 'package:kalam_link/kalam_link.dart';

Map<String, Object?> decode(Map<String, Object?> row) => row;
const table = KalamRemoteTable<Map<String, Object?>>(
  tableId: 'public.messages', keyColumn: 'id', decode: decode);

class FakeClient implements KalamClient {
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);

  @override
  Stream<List<T>> live<T>(String sql, {
    int? batchSize, int? lastRows, VersionId? from, String? subscriptionId,
    int? limit, List<String>? keyColumns,
    T Function(Map<String, KalamCellValue> row)? mapRow,
    void Function(LiveCheckpoint checkpoint)? onCheckpoint,
  }) => Stream.value([
    for (final n in [1, 3, 2]) mapRow!({
      'id': KalamCellValue('$n'), 'created_at': KalamCellValue(n),
    }),
  ]);
}

void main() {
  test('file name matches canonical server truncation before dash collapse', () {
    final name = 'a${' ' * 60}b.txt';
    final file = KalamFileRef(id:'123', sub:'f0001', name:name, size:1, mime:'text/plain', sha256:'abc');
    expect(file.storedName(), '123-a.txt');
  });

  test('chained where preserves the whole OR predicate', () {
    final query = table.plan().query()
      .where((c) => KalamPredicate.or([c.col('owner').eq('me'), c.col('public').eq(true)]))
      .where((c) => c.col('deleted').eq(false));
    expect(query.compile().sql,
      'SELECT * FROM "public"."messages" WHERE (("owner" = \$1) OR ("public" = \$2)) AND ("deleted" = \$3)');
  });

  test('live applies the requested order', () async {
    final rows = await table.bind(FakeClient()).query()
      .orderBy((c) => c.col('created_at').desc()).live().first;
    expect(rows.map((r) => r['created_at']), [3, 2, 1]);
  });

  test('query definition does not change when caller mutates input list', () {
    final keys = <Object?>['a'];
    final query = table.plan().query().where((c) => c.col('id').inList(keys));
    final before = query.compile().params.toList();
    keys.add('b');
    expect(query.compile().params, before);
  });
}
