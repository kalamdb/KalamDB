import 'package:drift_flutter/drift_flutter.dart';

import '../database/kalam_sync_database.dart';
import '../models/kalam_account_identity.dart';

abstract interface class KalamDatabaseFactory {
  Future<KalamSyncDatabase> open(KalamAccountIdentity identity);
}

final class KalamFlutterDatabaseFactory implements KalamDatabaseFactory {
  const KalamFlutterDatabaseFactory();

  @override
  Future<KalamSyncDatabase> open(KalamAccountIdentity identity) async {
    return KalamSyncDatabase(
      driftDatabase(
        name: identity.databaseName,
        native: const DriftNativeOptions(shareAcrossIsolates: true),
      ),
    );
  }
}
