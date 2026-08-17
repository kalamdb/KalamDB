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
  final SeqId seq;
  final Map<String, Object?> row;
  final Map<String, Object?>? oldRow;
  final bool initial;
}
