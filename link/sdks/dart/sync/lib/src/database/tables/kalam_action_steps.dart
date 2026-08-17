import 'package:drift/drift.dart';

@DataClassName('StoredActionStep')
class KalamActionSteps extends Table {
  TextColumn get actionId => text()();
  TextColumn get name => text()();
  TextColumn get status => text()();
  TextColumn get resultJson => text().nullable()();
  TextColumn get lastError => text().nullable()();
  DateTimeColumn get updatedAt => dateTime()();

  @override
  Set<Column<Object>> get primaryKey => {actionId, name};
}
