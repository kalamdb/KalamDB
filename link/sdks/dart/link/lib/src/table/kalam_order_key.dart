part of 'kalam_remote_table.dart';

final class KalamOrderKey {
  const KalamOrderKey(this.column, {required this.descending});

  final String column;
  final bool descending;
}
