import 'package:drift/drift.dart';

@DataClassName('StoredRowState')
class KalamRowStates extends Table {
  TextColumn get accountKey => text()();
  TextColumn get tableId => text()();
  TextColumn get rowKey => text()();
  TextColumn get phase => text()();
  TextColumn get actionId => text().nullable()();
  IntColumn get attemptCount => integer().withDefault(const Constant(0))();
  DateTimeColumn get nextRetryAt => dateTime().nullable()();
  TextColumn get errorCode => text().nullable()();
  TextColumn get errorMessage => text().nullable()();
  TextColumn get lastServerSeq => text().nullable()();
  TextColumn get pendingValuesJson => text().nullable()();
  BoolColumn get tombstone => boolean().withDefault(const Constant(false))();
  DateTimeColumn get updatedAt => dateTime()();

  @override
  Set<Column<Object>> get primaryKey => {accountKey, tableId, rowKey};
}
