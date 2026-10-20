import 'package:kalam_link/kalam_link.dart';

import '../models/kalam_change.dart';

/// One decoded row change delivered by a synchronization batch.
final class KalamRemoteChange {
  const KalamRemoteChange({
    required this.kind,
    required this.seq,
    required this.row,
    this.oldRow,
    this.initial = false,
  });

  final KalamChangeKind kind;
  final VersionId seq;
  final Map<String, Object?> row;
  final Map<String, Object?>? oldRow;
  final bool initial;

  bool get isPartialUpdate => kind == KalamChangeKind.update && !initial;

  /// Row columns plus `_version`, wrapped as cells.
  Map<String, KalamCellValue> get cells => {
        for (final entry in rowWithVersion.entries)
          entry.key: KalamCellValue.from(entry.value),
      };

  /// Row columns plus `_version`.
  ///
  /// [row] leaves the system column off so app code can treat it as table
  /// data. Decoders that expect the SQL shape, where `_version` sits on the
  /// row, should read this map. [seq] remains the typed version.
  Map<String, Object?> get rowWithVersion => {
        ...row,
        '_version': seq.value,
      };
}
