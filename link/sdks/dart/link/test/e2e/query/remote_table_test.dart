/// Remote table builder against a live KalamDB server.
///
/// Covers Slice 1: schema-qualified insert, get, filtered query, update,
/// delete, and live on the existing query and live calls.
library;

import 'package:flutter_test/flutter_test.dart';
import 'package:kalam_link/kalam_link.dart';

import '../helpers.dart';

final class _Note {
  const _Note({required this.id, required this.title});

  final String id;
  final String title;
}

void main() {
  group('Remote table', skip: skipIfNoIntegration, () {
    late KalamClient client;
    late String tableId;
    late KalamTableClient<_Note> notes;

    setUpAll(() async {
      client = await connectJwtClient();
      final namespace = uniqueName('dart_remote_table');
      tableId = '$namespace.notes';
      await ensureNamespace(client, namespace);
      final created = await client.query(
        'CREATE TABLE IF NOT EXISTS $tableId ('
        'id TEXT PRIMARY KEY, '
        'title TEXT NOT NULL'
        ')',
      );
      expect(created.success, isTrue, reason: created.error?.toString());
      notes = KalamRemoteTable<_Note>(
        tableId: tableId,
        keyColumn: 'id',
        decode: (json) => _Note(
          id: json['id']! as String,
          title: json['title']! as String,
        ),
      ).bind(client);
    });

    tearDownAll(() async {
      await dropTable(client, tableId);
      await client.dispose();
    });

    test('inserts, reads, updates, and deletes one row', () async {
      await notes.insert({'id': 'a', 'title': 'alpha'});

      final stored = await notes.get('a');
      expect(stored?.title, 'alpha');

      final matched = await notes
          .query()
          .where((columns) => columns.col('title').eq('alpha'))
          .get();
      expect(matched.map((row) => row.id), ['a']);

      await notes.update('a', {'title': 'beta'});
      expect((await notes.get('a'))?.title, 'beta');

      await notes.delete('a');
      expect(await notes.get('a'), isNull);
    });

    test('live emits the inserted row', () async {
      final seen = <List<_Note>>[];
      final subscription = notes
          .query()
          .where((columns) => columns.col('id').eq('live-1'))
          .live()
          .listen(seen.add);
      addTearDown(() => subscription.cancel().timeout(const Duration(seconds: 3)));

      await notes.insert({'id': 'live-1', 'title': 'live'});

      await waitForCondition(
        () => seen.any((batch) => batch.any((row) => row.title == 'live')),
        timeout: const Duration(seconds: 20),
      );
    });
  });
}
