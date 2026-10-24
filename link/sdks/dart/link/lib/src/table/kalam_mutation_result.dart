part of 'kalam_remote_table.dart';

final class KalamMutationResult {
  const KalamMutationResult({required this.rowCount, required this.response});

  final int rowCount;
  final QueryResponse response;
}
