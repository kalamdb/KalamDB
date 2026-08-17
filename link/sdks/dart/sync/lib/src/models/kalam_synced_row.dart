import 'kalam_row_sync_state.dart';

/// A Drift-generated row paired with SDK-owned local synchronization state.
final class KalamSyncedRow<T> {
  const KalamSyncedRow({required this.value, required this.sync});

  final T value;
  final KalamRowSyncState sync;

  bool get isSynced => sync.isSynced;

  @override
  bool operator ==(Object other) {
    return other is KalamSyncedRow<T> &&
        other.value == value &&
        other.sync == sync;
  }

  @override
  int get hashCode => Object.hash(value, sync);
}
