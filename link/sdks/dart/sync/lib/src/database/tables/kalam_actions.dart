import 'package:drift/drift.dart';

@DataClassName('StoredAction')
class KalamActions extends Table {
  TextColumn get id => text()();
  TextColumn get accountKey => text()();
  TextColumn get actionKey => text()();
  IntColumn get version => integer().withDefault(const Constant(1))();
  TextColumn get kind => text().withDefault(const Constant('custom'))();
  TextColumn get payloadJson => text()();
  TextColumn get status => text()();
  TextColumn get orderingKey => text().nullable()();
  TextColumn get rowTableId => text().nullable()();
  TextColumn get rowKey => text().nullable()();
  IntColumn get queuePosition => integer().withDefault(const Constant(0))();
  IntColumn get attemptCount => integer().withDefault(const Constant(0))();
  DateTimeColumn get nextAttemptAt => dateTime().nullable()();
  TextColumn get lastError => text().nullable()();
  DateTimeColumn get createdAt => dateTime()();
  DateTimeColumn get updatedAt => dateTime()();

  @override
  Set<Column<Object>> get primaryKey => {id};
}
