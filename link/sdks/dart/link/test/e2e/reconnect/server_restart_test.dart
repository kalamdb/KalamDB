import 'dart:async';
import 'dart:io';
import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:kalam_link/kalam_link.dart';
import 'package:kalam_link/src/generated/frb_generated.dart';
import '../helpers.dart';

Future<void> waitFor(bool Function() condition) async {
  final until = DateTime.now().add(const Duration(seconds: 120));
  while (!condition()) {
    if (DateTime.now().isAfter(until))
      throw TimeoutException('waiting for event');
    await Future<void>.delayed(const Duration(milliseconds: 50));
  }
}

void main() {
  test('Dart subscription recovers after server restart', () async {
    await RustLib.init(
        externalLibrary: ExternalLibrary.open(
            Platform.environment['KALAM_LINK_TEST_LIBRARY']!));
    final client = await connectJwtClient();
    final writer = await connectJwtClient();
    await writer.query('CREATE SCHEMA IF NOT EXISTS dart_restart');
    await writer.query(
        'CREATE TABLE IF NOT EXISTS dart_restart.items (id INT PRIMARY KEY, payload TEXT)');
    final events = <ChangeEvent>[];
    final subscription = client
        .liveEvents('SELECT * FROM dart_restart.items',
            subscriptionId: 'dart-restart')
        .listen(events.add);
    await waitFor(() => events.whereType<AckEvent>().isNotEmpty);
    await writer.query(
        "INSERT INTO dart_restart.items (id, payload) VALUES (100, 'before')");
    await waitFor(() => changeEventsContainRowId(events, 100));
    File('${Platform.environment['KALAM_RESTART_COORD_DIR']}/dart-ready')
        .writeAsStringSync('ready');
    await waitFor(() =>
        File('${Platform.environment['KALAM_RESTART_COORD_DIR']}/restarted')
            .existsSync());
    await waitFor(() => events.whereType<AckEvent>().length >= 2);
    await writer.query(
        "INSERT INTO dart_restart.items (id, payload) VALUES (200, 'after')");
    await waitFor(() => changeEventsContainRowId(events, 200));
    expect(await client.isConnected, isTrue);
    await subscription.cancel();
    await client.dispose();
    await writer.dispose();
  },
      skip: Platform.environment['KALAM_RESTART_COORD_DIR'] == null,
      timeout: const Timeout(Duration(seconds: 150)));
}
