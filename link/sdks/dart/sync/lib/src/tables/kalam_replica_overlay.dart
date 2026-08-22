import 'dart:convert';

import '../models/kalam_row_overlay.dart';
import '../models/kalam_row_sync_state.dart';
import '../models/kalam_synced_row.dart';
import 'kalam_table_spec.dart';

/// Pure merge of authoritative rows and SDK-owned optimistic row state.
final class KalamReplicaOverlay<T> {
  const KalamReplicaOverlay(this.spec);

  final KalamTableSpec<T> spec;

  List<KalamSyncedRow<T>> merge(
    List<T> backendRows,
    List<KalamRowOverlay> overlays,
  ) {
    final byKey = <String, T>{
      for (final row in backendRows) spec.keyOf(row): row,
    };
    final stateByKey = <String, KalamRowSyncState>{};

    for (final overlay in overlays) {
      stateByKey[overlay.rowKey] = overlay.sync;
      if (overlay.tombstone) {
        byKey.remove(overlay.rowKey);
      } else if (overlay.pendingValuesJson != null) {
        final json = jsonDecode(overlay.pendingValuesJson!);
        if (json is! Map<String, Object?>) {
          throw FormatException(
            'Pending values for ${spec.tableId}/${overlay.rowKey} '
            'must be a JSON object.',
          );
        }
        byKey[overlay.rowKey] = spec.decode(json);
      }
    }

    return [
      for (final entry in byKey.entries)
        KalamSyncedRow(
          value: entry.value,
          sync: stateByKey[entry.key] ?? const KalamRowSyncState.synced(),
        ),
    ];
  }
}
