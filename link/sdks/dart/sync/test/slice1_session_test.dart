import 'package:drift/native.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kalam_sync/drift.dart' hide isNotNull, isNull;
import 'package:kalam_sync/kalam_sync.dart';

void main() {
  test(
    'qualified spec sql and partial changes stay distinct from full rows',
    () {
      final spec = KalamTableSpec<String>(
        tableId: 'public.messages',
        keyColumn: 'id',
        mode: KalamSyncMode.replicaOnly,
        keyOf: (row) => row,
        encode: (row) => {'id': row},
        decode: (json) => json['id'] as String,
      );

      expect(
        spec.copyWith(subscriptionId: 'my-checkpoint').liveSql,
        spec.liveSql,
      );
      expect(
        () => spec.decodeChange(
          KalamRemoteChange(
            kind: KalamChangeKind.delete,
            seq: VersionId.parse('5'),
            row: const {},
          ),
        ),
        throwsFormatException,
      );
      expect(spec.bind(_UnusedClient()).table.decode({'id': 'm'}), 'm');
      expect(spec.liveSql, 'SELECT * FROM "public"."messages"');

      final full = spec.decodeChange(
        KalamRemoteChange(
          kind: KalamChangeKind.insert,
          seq: VersionId.parse('1'),
          row: const {'id': 'm1', 'body': 'hi'},
          initial: true,
        ),
      );
      final partial = spec.decodeChange(
        KalamRemoteChange(
          kind: KalamChangeKind.update,
          seq: VersionId.parse('2'),
          row: const {'id': 'm1', 'acks': '{}'},
        ),
      );
      final deleted = spec.decodeChange(
        KalamRemoteChange(
          kind: KalamChangeKind.delete,
          seq: VersionId.parse('3'),
          row: const {'id': 'm1'},
        ),
      );

      expect(full, isA<KalamFullRow<String>>());
      expect((partial as KalamPartialRow<String>).columns['acks'], '{}');
      expect((deleted as KalamDeletedRow<String>).key, 'm1');
      expect(
        KalamRemoteChange(
          kind: KalamChangeKind.update,
          seq: VersionId.parse('2'),
          row: const {'id': 'm1'},
        ).isPartialUpdate,
        isTrue,
      );
    },
  );

  test('the same user keeps the session', () {
    expect(
      planKalamSession(existingSubject: 'user-1', userId: 'user-1'),
      KalamSessionPlan.keep,
    );
    expect(
      planKalamSession(existingSubject: null, userId: 'user-1'),
      KalamSessionPlan.open,
    );
    expect(
      planKalamSession(existingSubject: 'user-1', userId: 'user-2'),
      KalamSessionPlan.replace,
    );
  });

  test('an attached database is not owned and a constructor is', () async {
    driftRuntimeOptions.dontWarnAboutMultipleDatabases = true;
    addTearDown(() {
      driftRuntimeOptions.dontWarnAboutMultipleDatabases = false;
    });
    final attached = KalamSyncDatabase(NativeDatabase.memory());
    addTearDown(attached.close);
    const identity = KalamAccountIdentity(
      serverUrl: 'http://db',
      subject: 'user-1',
    );
    final adopted = await resolveKalamDatabase(
      identity: identity,
      database: attached,
    );
    expect(adopted.owned, isFalse);
    expect(identical(adopted.database, attached), isTrue);

    final constructed = await resolveKalamDatabase(
      identity: identity,
      openExecutor: (_) async => NativeDatabase.memory(),
      databaseConstructor: KalamSyncDatabase.new,
    );
    addTearDown(constructed.database.close);
    expect(constructed.owned, isTrue);
    expect(
      () => resolveKalamDatabase(
        identity: identity,
        database: attached,
        databaseConstructor: KalamSyncDatabase.new,
      ),
      throwsArgumentError,
    );
  });

  test('deferred emit waits and emitNow cancels the wait', () async {
    final emitted = <String>[];
    final gate = KalamDeferredEmitter<String>(
      delay: const Duration(milliseconds: 30),
      onEmit: emitted.add,
    );
    addTearDown(gate.dispose);
    gate.defer('offline');
    expect(gate.hasPending, isTrue);
    gate.emitNow('live');
    await Future<void>.delayed(const Duration(milliseconds: 40));
    expect(emitted, ['live']);
  });
}

class _UnusedClient implements KalamClient {
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}
