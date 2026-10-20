import 'package:kalam_link/kalam_link.dart';

import '../models/kalam_change.dart';
import '../models/kalam_sync_mode.dart';
import '../transport/kalam_remote_change.dart';

/// Generated metadata and codecs for one Drift-owned application table.
final class KalamTableSpec<T> {
  const KalamTableSpec({
    required this.tableId,
    required this.keyColumn,
    required this.mode,
    required this.keyOf,
    required this.encode,
    required this.decode,
    String? subscriptionId,
  }) : subscriptionId = subscriptionId ?? tableId;

  final String tableId;
  final String keyColumn;
  final String subscriptionId;
  final KalamSyncMode mode;
  final String Function(T row) keyOf;
  final Map<String, Object?> Function(T row) encode;
  final T Function(Map<String, Object?> json) decode;

  /// `SELECT *` for this subscription. Drift SQL is never used here.
  String get liveSql => 'SELECT * FROM ${quoteKalamTable(tableId)}';

  /// Reuses the generated codecs and key for remote queries and live views.
  KalamTableClient<T> bind(KalamClient client) => KalamRemoteTable<T>(
    tableId: tableId,
    keyColumn: keyColumn,
    decode: decode,
  ).bind(client);

  KalamDecodedRow<T> decodeChange(KalamRemoteChange change) {
    if (change.kind == KalamChangeKind.delete) {
      final key = change.row[keyColumn];
      if (key == null)
        throw FormatException('Delete event is missing $keyColumn');
      return KalamDeletedRow(key.toString());
    }
    if (change.isPartialUpdate) {
      return KalamPartialRow(Map<String, Object?>.from(change.row));
    }
    return KalamFullRow(decode(change.row));
  }

  KalamTableSpec<T> copyWith({KalamSyncMode? mode, String? subscriptionId}) {
    return KalamTableSpec(
      tableId: tableId,
      keyColumn: keyColumn,
      mode: mode ?? this.mode,
      keyOf: keyOf,
      encode: encode,
      decode: decode,
      subscriptionId: subscriptionId ?? this.subscriptionId,
    );
  }
}

sealed class KalamDecodedRow<T> {
  const KalamDecodedRow();
}

final class KalamFullRow<T> extends KalamDecodedRow<T> {
  const KalamFullRow(this.row);

  final T row;
}

final class KalamPartialRow<T> extends KalamDecodedRow<T> {
  const KalamPartialRow(this.columns);

  final Map<String, Object?> columns;
}

final class KalamDeletedRow<T> extends KalamDecodedRow<T> {
  const KalamDeletedRow(this.key);

  final String key;
}
