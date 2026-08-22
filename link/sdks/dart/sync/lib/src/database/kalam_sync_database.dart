import 'package:drift/drift.dart';

import 'tables/kalam_action_steps.dart';
import 'tables/kalam_actions.dart';
import 'tables/kalam_checkpoints.dart';
import 'tables/kalam_cached_rows.dart';
import 'tables/kalam_row_states.dart';

part 'kalam_sync_database.g.dart';

@DriftDatabase(
  tables: [
    KalamActions,
    KalamCachedRows,
    KalamActionSteps,
    KalamCheckpoints,
    KalamRowStates,
  ],
)
class KalamSyncDatabase extends _$KalamSyncDatabase {
  KalamSyncDatabase(super.executor);

  @override
  int get schemaVersion => 3;

  @override
  MigrationStrategy get migration => MigrationStrategy(
    onCreate: (migrator) => migrator.createAll(),
    onUpgrade: (migrator, from, to) async {
      if (from < 2) await migrator.createTable(kalamCachedRows);
      if (from < 3) {
        await migrator.addColumn(kalamActions, kalamActions.queuePosition);
        await customStatement(
          'UPDATE kalam_actions SET queue_position = rowid '
          'WHERE queue_position = 0',
        );
      }
    },
  );
}
