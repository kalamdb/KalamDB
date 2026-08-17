import 'dart:async';

import 'package:drift/native.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kalam_sync/drift.dart' hide isNotNull, isNull;
import 'package:kalam_sync/kalam_sync.dart';

final class ScopeTransport implements KalamSyncTransport {
  final connectionController =
      StreamController<KalamTransportConnection>.broadcast();
  var pauseCount = 0;
  var resumeCount = 0;

  @override
  Stream<KalamTransportConnection> get connectionStates =>
      connectionController.stream;

  @override
  Future<void> ensureConnected() async {}

  @override
  Stream<KalamRemoteBatch> subscribe({
    required String sql,
    required String subscriptionId,
    SeqId? from,
    int? batchSize,
  }) => const Stream.empty();

  @override
  Future<void> pause() async => pauseCount++;

  @override
  Future<void> resume() async => resumeCount++;

  @override
  Future<void> dispose() => connectionController.close();
}

void main() {
  late Kalam kalam;
  late ScopeTransport transport;

  setUp(() {
    transport = ScopeTransport();
    kalam = Kalam.fromComponents(
      identity: const KalamAccountIdentity(
        serverUrl: 'https://db.example.com',
        subject: 'user-a',
      ),
      database: KalamSyncDatabase(NativeDatabase.memory()),
      transport: transport,
    );
  });

  tearDown(() => kalam.dispose());

  testWidgets('provides the account-scoped Kalam session', (tester) async {
    late Kalam found;
    await tester.pumpWidget(
      KalamScope(
        kalam: kalam,
        child: Builder(
          builder: (context) {
            found = KalamScope.of(context);
            return const SizedBox();
          },
        ),
      ),
    );

    expect(found, same(kalam));
  });

  testWidgets('reads the session during widget initialization', (tester) async {
    late Kalam found;
    await tester.pumpWidget(
      KalamScope(
        kalam: kalam,
        child: _ReadOnInit(onRead: (value) => found = value),
      ),
    );

    expect(found, same(kalam));
  });

  testWidgets('pauses and resumes sync with the app lifecycle', (tester) async {
    await tester.pumpWidget(KalamScope(kalam: kalam, child: const SizedBox()));

    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.paused);
    await tester.pump();
    expect(transport.pauseCount, 1);
    expect(kalam.syncState.phase, KalamSyncPhase.paused);

    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
    await tester.pump();
    expect(transport.resumeCount, 1);
  });

  test('database identity changes with server, namespace, or subject', () {
    const first = KalamAccountIdentity(
      serverUrl: 'https://db.example.com',
      subject: 'user-a',
    );
    const second = KalamAccountIdentity(
      serverUrl: 'https://db.example.com',
      subject: 'user-b',
    );

    expect(first.databaseName, isNot(second.databaseName));
    expect(first.databaseName, startsWith('kalam_sync_'));
    expect(
      first.databaseName,
      KalamAccountIdentity(
        serverUrl: first.serverUrl,
        subject: first.subject,
      ).databaseName,
    );
  });

  test('generates RFC 4122 version 4 action ids', () {
    final first = Kalam.id();
    final second = Kalam.id();

    expect(
      first,
      matches(
        RegExp(
          r'^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$',
        ),
      ),
    );
    expect(second, isNot(first));
  });
}

final class _ReadOnInit extends StatefulWidget {
  const _ReadOnInit({required this.onRead});

  final ValueChanged<Kalam> onRead;

  @override
  State<_ReadOnInit> createState() => _ReadOnInitState();
}

final class _ReadOnInitState extends State<_ReadOnInit> {
  @override
  void initState() {
    super.initState();
    widget.onRead(KalamScope.read(context));
  }

  @override
  Widget build(BuildContext context) => const SizedBox();
}
