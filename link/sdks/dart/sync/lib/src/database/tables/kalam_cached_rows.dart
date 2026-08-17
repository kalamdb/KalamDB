import 'package:drift/drift.dart';

@DataClassName('StoredCachedRow')
class KalamCachedRows extends Table {
  TextColumn get accountKey => text()();
  TextColumn get tableId => text()();
  TextColumn get rowKey => text()();
  TextColumn get valuesJson => text()();
  DateTimeColumn get updatedAt => dateTime()();

  @override
  Set<Column<Object>> get primaryKey => {accountKey, tableId, rowKey};
}
