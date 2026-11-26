import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kalam_link/kalam_link.dart';
import 'package:kalam_link/src/generated/frb_generated.dart';

const _deadline = Duration(seconds: 5);

void _ack(WebSocket socket, String id) {
  socket.add(jsonEncode({
    'type': 'subscription_ack',
    'subscription_id': id,
    'total_rows': 0,
    'schema': [],
    'batch_control': {'status': 'ready', 'batch_num': 0, 'has_more': false},
  }));
}

void _insert(WebSocket socket, String id, List<int> versions) {
  socket.add(jsonEncode({
    'type': 'change',
    'change_type': 'insert',
    'subscription_id': id,
    'rows': versions
        .map((version) => {'id': version.toString(), '_version': version})
        .toList(),
  }));
}

void main() {
  final libraryPath = Platform.environment['KALAM_LINK_TEST_LIBRARY'];

  test('native bridge reauthenticates and resumes from the acknowledged cursor',
      () async {
    await RustLib.init(externalLibrary: ExternalLibrary.open(libraryPath!));
    final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    final sockets = <WebSocket>[];
    final tokens = <String?>[];
    final requests = List.generate(3, (_) => Completer<Map<String, dynamic>>());
    var requestCount = 0;

    server.listen((request) async {
      tokens.add(request.headers.value(HttpHeaders.authorizationHeader));
      final socket = await WebSocketTransformer.upgrade(request);
      sockets.add(socket);
      socket.add(jsonEncode({
        'type': 'auth_success',
        'user': 'fixture',
        'role': 'user',
        'protocol': {'serialization': 'json', 'compression': 'none'},
      }));
      socket.listen((message) {
        if (message is! String) return;
        final frame = jsonDecode(message) as Map<String, dynamic>;
        if (frame['type'] != 'subscribe') return;
        final subscription = frame['subscription'] as Map<String, dynamic>;
        if (requestCount < requests.length) {
          requests[requestCount++].complete(subscription);
        }
      });
    });
    addTearDown(() async {
      for (final socket in sockets) {
        await socket.close();
      }
      await server.close(force: true);
    });

    var authCalls = 0;
    final client = await KalamClient.connect(
      url: 'http://127.0.0.1:${server.port}',
      authProvider: () async => Auth.jwt('fixture-token-${++authCalls}'),
      wsLazyConnect: false,
    ).timeout(_deadline);
    addTearDown(client.dispose);
    final events =
        StreamIterator(client.liveEventsWithAck('SELECT id FROM fixture'));
    addTearDown(events.cancel);

    final readAck = events.moveNext();
    final first = await requests[0].future.timeout(_deadline);
    final id = first['id'] as String;
    _ack(sockets.first, id);
    expect(await readAck.timeout(_deadline), isTrue);
    expect(events.current.event, isA<AckEvent>());

    _insert(sockets.first, id, [42]);
    expect(await events.moveNext().timeout(_deadline), isTrue);
    expect(events.current.event, isA<InsertEvent>());
    await events.current.acknowledge();

    _insert(sockets.first, id, [43]);
    expect(await events.moveNext().timeout(_deadline), isTrue);
    expect(events.current.event, isA<InsertEvent>());

    await sockets.first.close(WebSocketStatus.goingAway, 'server restart');
    final second = await requests[1].future.timeout(_deadline);
    expect(second['id'], id);
    expect(second['options']['from'], '42');
    expect(tokens, ['Bearer fixture-token-1', 'Bearer fixture-token-2']);
    expect(await client.isConnected, isFalse);

    final ready = client.reconnectWebSocket();
    _ack(sockets.last, id);
    await ready.timeout(_deadline);
    expect(sockets, hasLength(2));
    expect(await client.isConnected, isTrue);

    // The outstanding delivery blocks the Dart stream, but recovery can finish
    // before the consumer acknowledges it.
    await events.current.acknowledge();
    expect(await events.moveNext().timeout(_deadline), isTrue);
    expect(events.current.event, isA<AckEvent>());

    _insert(sockets.last, id, [42, 43, 44]);
    expect(await events.moveNext().timeout(_deadline), isTrue);
    final replayed = events.current.event as InsertEvent;
    expect(replayed.rows.map((row) => row['id']?.asString()), ['44']);
    await events.current.acknowledge();

    sockets.last.add(jsonEncode({
      'type': 'error',
      'subscription_id': id,
      'code': 'UNSUPPORTED',
      'message': 'complete-event history expired; resynchronize',
    }));
    final snapshot = await requests[2].future.timeout(_deadline);
    expect(snapshot['id'], id);
    expect(snapshot['options']['from'], isNull);
    _ack(sockets.last, id);
    expect(await events.moveNext().timeout(_deadline), isTrue);
    expect(events.current.event, isA<AckEvent>());
    _insert(sockets.last, id, [7]);
    expect(await events.moveNext().timeout(_deadline), isTrue);
    expect((events.current.event as InsertEvent).row['id']?.asString(), '7');
  },
      skip: libraryPath == null
          ? 'Set KALAM_LINK_TEST_LIBRARY to a built FFI library'
          : false);
}
