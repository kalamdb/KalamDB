/// One server-shaped JSON row in Kalam's local mirror.
final class KalamCachedRow {
  const KalamCachedRow({required this.rowKey, required this.valuesJson});

  final String rowKey;
  final String valuesJson;
}
