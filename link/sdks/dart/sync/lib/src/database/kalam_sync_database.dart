import 'package:drift/drift.dart';

import 'tables/kalam_action_steps.dart';
import 'tables/kalam_actions.dart';
import 'tables/kalam_checkpoints.dart';
import 'tables/kalam_cached_rows.dart';
import 'tables/kalam_row_states.dart';

part 'kalam_sync_database.g.dart';

/// Sync tables an application database must expose on its one Drift connection.
abstract interface class KalamSyncTables {
  $KalamActionsTable get kalamActions;
  $KalamCachedRowsTable get kalamCachedRows;
  $KalamActionStepsTable get kalamActionSteps;
  $KalamCheckpointsTable get kalamCheckpoints;
  $KalamRowStatesTable get kalamRowStates;
}

/// Adds the sync tables to an application [GeneratedDatabase].
///
/// Include [kalamSyncSchema] in that database's `allSchemaEntities` and create
/// the tables from its migration. Kalam then uses the same connection.
mixin KalamSyncTablesMixin on GeneratedDatabase implements KalamSyncTables {
  @override
  late final $KalamActionsTable kalamActions = $KalamActionsTable(this);
  @override
  late final $KalamCachedRowsTable kalamCachedRows = $KalamCachedRowsTable(
    this,
  );
  @override
  late final $KalamActionStepsTable kalamActionSteps = $KalamActionStepsTable(
    this,
  );
  @override
  late final $KalamCheckpointsTable kalamCheckpoints = $KalamCheckpointsTable(
    this,
  );
  @override
  late final $KalamRowStatesTable kalamRowStates = $KalamRowStatesTable(this);

  List<DatabaseSchemaEntity> get kalamSyncSchema => [
    kalamActions,
    kalamCachedRows,
    kalamActionSteps,
    kalamCheckpoints,
    kalamRowStates,
  ];
}

@DriftDatabase(
  tables: [
    KalamActions,
    KalamCachedRows,
    KalamActionSteps,
    KalamCheckpoints,
    KalamRowStates,
  ],
)
class KalamSyncDatabase extends _$KalamSyncDatabase implements KalamSyncTables {
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
