/// Simple example showing queries, login, token refresh, and live events.
// ignore_for_file: avoid_print
library;

import 'dart:async';

import 'package:kalam_link/kalam_link.dart';

import '../support.dart';

Future<void> main() async {
  await runSimpleEventsExample();
}

Future<void> runSimpleEventsExample({String namespace = 'sdk_examples'}) async {
  final fullTable = '$namespace.tasks';
  final session = await connectAdminSession();

  try {
    print('Logged in as ${session.login.user.id} '
        '(${session.login.user.role.name})');

    await queryOrThrow(
      session.client,
      'CREATE NAMESPACE IF NOT EXISTS $namespace',
    );
    await queryOrThrow(session.client, 'DROP TABLE IF EXISTS $fullTable');
    await queryOrThrow(
      session.client,
      'CREATE TABLE $fullTable '
      '(id INT PRIMARY KEY, title TEXT, done BOOLEAN)',
    );

    await queryOrThrow(
      session.client,
      'INSERT INTO $fullTable (id, title, done) VALUES (\$1, \$2, \$3)',
      params: [1, 'Buy groceries', false],
    );
    await queryOrThrow(
      session.client,
      'INSERT INTO $fullTable (id, title, done) VALUES (\$1, \$2, \$3)',
      params: [2, 'Write tests', true],
    );

    final selected = await queryOrThrow(
      session.client,
      'SELECT * FROM $fullTable ORDER BY id',
    );
    print('Current tasks:');
    print(describeRows(selected.rows));

    if (session.login.refreshToken != null) {
      final refreshed =
          await session.anonClient.refreshToken(session.login.refreshToken!);
      print('Refreshed token expires at ${refreshed.expiresAt}');
    }

    final ackSeen = Completer<void>();
    final insertSeen = Completer<void>();
    late final StreamSubscription<ChangeEvent> subscription;
    subscription = session.client.liveEvents('SELECT * FROM $fullTable').listen(
      (event) {
        switch (event) {
          case AckEvent(:final totalRows, :final schema):
            print('Subscription ack: $totalRows rows, ${schema.length} columns');
            if (!ackSeen.isCompleted) {
              ackSeen.complete();
            }
          case InitialDataBatch(:final rows, :final hasMore):
            print('Initial snapshot (${rows.length} rows, more=$hasMore)');
          case InsertEvent(:final row):
            print('Inserted row: ${describeRow(row)}');
            if (row['id']?.asInt() == 3 && !insertSeen.isCompleted) {
              insertSeen.complete();
            }
          case UpdateEvent(:final row, :final oldRow):
            print(
              'Updated row: ${describeOptionalRow(oldRow)} -> ${describeRow(row)}',
            );
          case DeleteEvent(:final row):
            print('Deleted row: ${describeRow(row)}');
          case SubscriptionError(:final code, :final message):
            throw StateError('Subscription error [$code]: $message');
        }
      },
    );

    try {
      await ackSeen.future.timeout(const Duration(seconds: 10));
      await queryOrThrow(
        session.client,
        'INSERT INTO $fullTable (id, title, done) VALUES (\$1, \$2, \$3)',
        params: [3, 'Watch live events', false],
      );
      await insertSeen.future.timeout(const Duration(seconds: 10));
    } finally {
      await subscription.cancel();
    }

    print('Simple events example finished.');
  } finally {
    await session.dispose();
  }
}