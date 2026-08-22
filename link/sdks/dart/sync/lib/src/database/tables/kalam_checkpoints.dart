import 'package:drift/drift.dart';

@DataClassName('StoredCheckpoint')
class KalamCheckpoints extends Table {
  TextColumn get accountKey => text()();
  TextColumn get subscriptionId => text()();
  TextColumn get seq => text()();
  DateTimeColumn get updatedAt => dateTime()();

  @override
  Set<Column<Object>> get primaryKey => {accountKey, subscriptionId};
}
