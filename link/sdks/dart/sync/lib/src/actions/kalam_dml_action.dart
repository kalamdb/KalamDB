import 'package:kalam_link/kalam_link.dart';

import '../models/kalam_dml_payload.dart';
import '../models/kalam_retry_policy.dart';
import 'kalam_action_codec.dart';
import 'kalam_action_context.dart';
import 'kalam_action_definition.dart';

const kalamDmlActionKey = 'kalam.dml';

KalamActionDefinition<KalamDmlPayload> kalamDmlAction(
  Future<KalamClient> Function() client,
) {
  return KalamActionDefinition(
    key: kalamDmlActionKey,
    codec: const KalamActionCodec(encode: _encodeDml, decode: _decodeDml),
    execute: (context, payload) async {
      await _executeDml(await client(), context, payload);
    },
  );
}

Map<String, Object?> _encodeDml(KalamDmlPayload payload) => {
  'operation': payload.operation.name,
  'table': payload.tableId,
  'keyColumn': payload.keyColumn,
  'rowKey': payload.rowKey,
  'values': payload.values,
};

KalamDmlPayload _decodeDml(Map<String, Object?> json) {
  final operation = KalamDmlOperation.values.byName(
    json['operation']! as String,
  );
  final values = json['values'];
  if (values != null && values is! Map) {
    throw const FormatException('DML values must be a map.');
  }
  if (operation != KalamDmlOperation.delete && values == null) {
    throw const FormatException('DML values are required for writes.');
  }
  final decodedValues = values is Map
      ? Map<String, Object?>.from(values)
      : const <String, Object?>{};
  return KalamDmlPayload(
    operation: operation,
    tableId: json['table']! as String,
    keyColumn: json['keyColumn']! as String,
    rowKey: json['rowKey'],
    values: decodedValues,
  );
}

Future<void> _executeDml(
  KalamClient client,
  KalamActionContext context,
  KalamDmlPayload payload,
) async {
  final table = _identifier(payload.tableId);
  final keyColumn = _identifier(payload.keyColumn);
  late String sql;
  late List<Object?> params;

  switch (payload.operation) {
    case KalamDmlOperation.insert:
      final columns = payload.values.keys.map(_identifier).toList();
      if (columns.isEmpty) {
        throw const KalamPermanentActionException('Insert values are empty.');
      }
      final placeholders = List.generate(
        columns.length,
        (index) => '\$${index + 1}',
      );
      sql =
          'INSERT INTO $table (${columns.join(', ')}) '
          'VALUES (${placeholders.join(', ')})';
      params = [for (final key in payload.values.keys) payload.values[key]];
    case KalamDmlOperation.update:
      final entries = payload.values.entries
          .where((entry) => entry.key != payload.keyColumn)
          .toList();
      if (entries.isEmpty) {
        throw const KalamPermanentActionException('Update values are empty.');
      }
      final assignments = [
        for (var index = 0; index < entries.length; index++)
          '${_identifier(entries[index].key)} = \$${index + 1}',
      ];
      sql =
          'UPDATE $table SET ${assignments.join(', ')} '
          'WHERE $keyColumn = \$${entries.length + 1}';
      params = [...entries.map((entry) => entry.value), payload.rowKey];
    case KalamDmlOperation.delete:
      sql = 'DELETE FROM $table WHERE $keyColumn = \$1';
      params = [payload.rowKey];
  }

  final response = await client.query(sql, params: params);
  if (!response.success) {
    final error = response.error;
    throw KalamPermanentActionException(
      error == null ? 'KalamDB rejected the mutation.' : error.toString(),
    );
  }
}

String _identifier(String value) {
  final parts = value.split('.');
  final valid = RegExp(r'^[A-Za-z_][A-Za-z0-9_]*$');
  if (parts.isEmpty || parts.any((part) => !valid.hasMatch(part))) {
    throw KalamPermanentActionException('Unsafe SQL identifier "$value".');
  }
  return parts.join('.');
}
