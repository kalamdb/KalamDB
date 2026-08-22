import 'kalam_row_sync_state.dart';

/// Internal overlay data used to combine backend rows with local intent.
final class KalamRowOverlay {
  const KalamRowOverlay({
    required this.rowKey,
    required this.sync,
    required this.tombstone,
    this.pendingValuesJson,
  });

  final String rowKey;
  final KalamRowSyncState sync;
  final String? pendingValuesJson;
  final bool tombstone;
}
