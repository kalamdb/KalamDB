import 'package:kalam_link/kalam_link.dart';

enum KalamChangeKind { insert, update, delete }

/// One ordered backend table change ready to apply to the local cache.
final class KalamChange<T> {
  const KalamChange({
    required this.kind,
    required this.rowKey,
    required this.seq,
    this.row,
  });

  final KalamChangeKind kind;
  final String rowKey;
  final SeqId seq;
  final T? row;
}
