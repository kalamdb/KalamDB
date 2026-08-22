import 'dart:io';

import 'package:drift/native.dart';
import 'package:kalam_sync/drift.dart' hide isNotNull, isNull;
import 'package:flutter_test/flutter_test.dart';
import 'package:kalam_sync/kalam_sync.dart';

void main() {
  test(
    'offline action survives restart with the same idempotency key',
    () async {
      final directory = await Directory.systemTemp.createTemp(
        'kalam_sync_test_',
      );
      final file = File('${directory.path}/sync.sqlite');
      final now = DateTime.utc(2026, 8, 16, 12);
      var database = KalamSyncDatabase(NativeDatabase(file));
      var store = KalamSyncStore(database, clock: () => now);

      await store.enqueue(
        const KalamActionDraft(
          id: 'stable-action-id',
          accountKey: 'server/user-a',
          actionKey: 'messages.send',
          payloadJson: '{"text":"hello"}',
        ),
        optimisticRow: const KalamOptimisticRow(
          tableId: 'app.messages',
          rowKey: 'message-1',
          phase: KalamRowSyncPhase.pending,
          pendingValuesJson: '{"id":"message-1","text":"hello"}',
        ),
      );
      await database.close();

      database = KalamSyncDatabase(NativeDatabase(file));
      store = KalamSyncStore(database, clock: () => now);
      final seenKeys = <String>[];
      final runner = KalamActionRunner(
        accountKey: 'server/user-a',
        store: store,
        registry: KalamActionRegistry([
          KalamActionDefinition<Map<String, Object?>>(
            key: 'messages.send',
            codec: KalamActionCodec(
              encode: (payload) => payload,
              decode: (json) => json,
            ),
            execute: (context, _) async => seenKeys.add(context.idempotencyKey),
          ),
        ]),
        clock: () => now,
      );

      expect(await runner.flush(), 1);
      expect(seenKeys, ['stable-action-id']);
      expect(
        (await store.readAction('stable-action-id'))?.status,
        KalamActionStatus.succeeded,
      );
      final overlays = await store
          .watchRowOverlays(
            accountKey: 'server/user-a',
            tableId: 'app.messages',
          )
          .first;
      expect(overlays.single.sync.phase, KalamRowSyncPhase.awaitingServerEcho);

      await database.close();
      await directory.delete(recursive: true);
    },
  );
}
