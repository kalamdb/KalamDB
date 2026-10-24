import 'package:flutter_test/flutter_test.dart';
import 'package:kalam_sync/kalam_sync.dart';
import '../lib/kalam.dart';

void main() {
  test('generated arrays, dates, enums and composites round trip', () {
    final row = Inventory.fromJson({
      'id': 'display-id', 'sku': 'actual-key', 'tags': ['a', 'b'],
      'dates': ['2026-10-19T00:00:00.000Z'], 'moods': ['calm'],
      'addresses': [{'city': 'Haifa'}], 'attachment': null,
    });
    expect(row.kalamRowKey, 'actual-key');
    expect(row.tags, ['a', 'b']);
    expect(row.dates!.single, DateTime.utc(2026, 10, 19));
    expect(row.addresses!.single.city, 'Haifa');
    expect(row.toJson()['moods'], ['calm']);
    expect(row.toJson()['dates'], ['2026-10-19T00:00:00.000Z']);
    expect(Inventory.fromJson(row.toJson()).tags, ['a', 'b']);
    expect(Inventory.fromJson({'id':'i', 'sku':'s'}).tags, isNull);
    expect(() => Inventory.fromJson({'id':'i', 'sku':'s', 'tags': 42}), throwsFormatException);
  });
  test('generated typed columns compile without a client or strings', () {
    final table = KalamRemoteTable<Inventory>(tableId: KalamTables.inventory.tableId,
      keyColumn: KalamTables.inventory.keyColumn, decode: Inventory.fromJson);
    final query = table.plan().query().filter(InventoryColumns.sku.eq('sku-1')).compile();
    expect(query.params, ['sku-1']);
    expect(query.sql, contains('"sku" = \$1'));
  });
}
