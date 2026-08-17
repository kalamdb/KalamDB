import 'dart:async';
import 'dart:convert';

import 'package:drift/native.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kalam_sync/drift.dart' hide isNotNull, isNull;
import 'package:kalam_sync/kalam_sync.dart';

final class Message {
  const Message({required this.id, required this.text});

  final String id;
  final String text;

  @override
  bool operator ==(Object other) =>
      other is Message && other.id == id && other.text == text;

  @override
  int get hashCode => Object.hash(id, text);
}

void main() {
  late KalamSyncDatabase database;
  late KalamSyncStore store;
  late List<Message> localRows;
  late StreamController<List<Message>> localChanges;
  final now = DateTime.utc(2026, 8, 16, 12);

  setUp(() {
    database = KalamSyncDatabase(NativeDatabase.memory());
    store = KalamSyncStore(database, clock: () => now);
    localRows = [];
    localChanges = StreamController.broadcast(onListen: () {});
  });

  tearDown(() async {
    await localChanges.close();
    await database.close();
  });

  KalamTableBinding<Message> binding(KalamSyncMode mode) {
    final spec = KalamTableSpec<Message>(
      tableId: 'app.messages',
      keyColumn: 'id',
      subscriptionId: 'conversation-1/messages',
      mode: mode,
      keyOf: (row) => row.id,
      encode: (row) => {'id': row.id, 'text': row.text},
      decode: (json) =>
          Message(id: json['id']! as String, text: json['text']! as String),
    );
    return KalamTableBinding(
      spec: spec,
      accountKey: 'server/user-a',
      store: store,
      watchLocal: () async* {
        yield List.of(localRows);
        yield* localChanges.stream;
      },
      upsertLocal: (message) async {
        localRows.removeWhere((row) => row.id == message.id);
        localRows.add(message);
        localChanges.add(List.of(localRows));
      },
      deleteLocal: (rowKey) async {
        localRows.removeWhere((row) => row.id == rowKey);
        localChanges.add(List.of(localRows));
      },
    );
  }

  test(
    'bidirectional insert writes locally and enqueues one generic DML action',
    () async {
      final messages = binding(KalamSyncMode.bidirectional);

      final action = await messages.insert(
        const Message(id: 'message-1', text: 'hello'),
        actionId: 'action-1',
      );

      expect(localRows, [const Message(id: 'message-1', text: 'hello')]);
      expect(action.actionKey, KalamTableBinding.dmlActionKey);
      expect(action.kind, 'dml');
      expect(
        jsonDecode(action.payloadJson),
        containsPair('operation', 'insert'),
      );
      expect(await database.select(database.kalamActions).get(), hasLength(1));
    },
  );

  test(
    'default binding keeps mirrored rows in Kalam-owned Drift storage',
    () async {
      final spec = KalamTableSpec<Message>(
        tableId: 'app.messages',
        keyColumn: 'id',
        mode: KalamSyncMode.bidirectional,
        keyOf: (row) => row.id,
        encode: (row) => {'id': row.id, 'text': row.text},
        decode: (json) =>
            Message(id: json['id']! as String, text: json['text']! as String),
      );
      final messages = KalamTableBinding(
        spec: spec,
        accountKey: 'server/user-a',
        store: store,
      );

      await messages.insert(
        const Message(id: 'message-1', text: 'stored once'),
        actionId: 'action-1',
      );

      expect(await messages.watch().first, [
        const Message(id: 'message-1', text: 'stored once'),
      ]);
      expect(
        await database.select(database.kalamCachedRows).get(),
        hasLength(1),
      );
      expect(await database.select(database.kalamActions).get(), hasLength(1));
    },
  );

  test('replica-only tables reject direct writes', () async {
    final messages = binding(KalamSyncMode.replicaOnly);

    expect(
      () => messages.insert(
        const Message(id: 'message-1', text: 'hello'),
        actionId: 'action-1',
      ),
      throwsStateError,
    );
  });

  test('optimistic replica row is visible with pending sync state', () async {
    final messages = binding(KalamSyncMode.replicaOnly);
    final pending = messages.watchWithSyncState().firstWhere(
      (rows) => rows.isNotEmpty,
    );

    await store.enqueue(
      const KalamActionDraft(
        id: 'action-1',
        accountKey: 'server/user-a',
        actionKey: 'messages.send',
        payloadJson: '{}',
      ),
      optimisticRow: const KalamOptimisticRow(
        tableId: 'app.messages',
        rowKey: 'message-1',
        phase: KalamRowSyncPhase.pending,
        pendingValuesJson: '{"id":"message-1","text":"hello"}',
      ),
    );

    final rows = await pending;
    expect(rows.single.value, const Message(id: 'message-1', text: 'hello'));
    expect(rows.single.sync.phase, KalamRowSyncPhase.pending);
  });

  test('pending delete hides an authoritative row', () async {
    localRows.add(const Message(id: 'message-1', text: 'hello'));
    final messages = binding(KalamSyncMode.replicaOnly);
    final hidden = messages.watch().firstWhere((rows) => rows.isEmpty);

    await store.enqueue(
      const KalamActionDraft(
        id: 'action-1',
        accountKey: 'server/user-a',
        actionKey: 'messages.delete',
        payloadJson: '{}',
      ),
      optimisticRow: const KalamOptimisticRow(
        tableId: 'app.messages',
        rowKey: 'message-1',
        phase: KalamRowSyncPhase.pendingDelete,
        tombstone: true,
      ),
    );

    expect(await hidden, isEmpty);
  });

  test('a stale row echo cannot complete a pending delete', () async {
    final messages = binding(KalamSyncMode.bidirectional);
    await messages.insert(
      const Message(id: 'message-1', text: 'original'),
      actionId: 'insert-1',
    );
    await messages.applyServerChange(
      const KalamChange(
        kind: KalamChangeKind.insert,
        rowKey: 'message-1',
        row: Message(id: 'message-1', text: 'original'),
        seq: SeqId(1),
      ),
    );

    await messages.delete('message-1', actionId: 'delete-1');
    await messages.applyServerChange(
      const KalamChange(
        kind: KalamChangeKind.update,
        rowKey: 'message-1',
        row: Message(id: 'message-1', text: 'stale echo'),
        seq: SeqId(2),
      ),
    );

    expect(
      (await store.readAction('delete-1'))?.status,
      KalamActionStatus.queued,
    );
    expect(await messages.watch().first, isEmpty);

    await messages.applyServerChange(
      const KalamChange(
        kind: KalamChangeKind.delete,
        rowKey: 'message-1',
        seq: SeqId(3),
      ),
    );
    expect(
      (await store.readAction('delete-1'))?.status,
      KalamActionStatus.succeeded,
    );
  });

  test('failed action is reflected beside its optimistic row', () async {
    final messages = binding(KalamSyncMode.replicaOnly);
    final failed = messages.watchWithSyncState().firstWhere(
      (rows) => rows.singleOrNull?.sync.phase == KalamRowSyncPhase.failed,
    );
    await store.enqueue(
      const KalamActionDraft(
        id: 'action-1',
        accountKey: 'server/user-a',
        actionKey: 'messages.send',
        payloadJson: '{}',
      ),
      optimisticRow: const KalamOptimisticRow(
        tableId: 'app.messages',
        rowKey: 'message-1',
        phase: KalamRowSyncPhase.pending,
        pendingValuesJson: '{"id":"message-1","text":"hello"}',
      ),
    );
    await store.failAction('action-1', 'invalid recipient', now);

    final row = (await failed).single;
    expect(row.sync.errorMessage, 'invalid recipient');
  });

  test(
    'server echo replaces overlay and commits the resume checkpoint',
    () async {
      final messages = binding(KalamSyncMode.replicaOnly);
      await store.enqueue(
        const KalamActionDraft(
          id: 'action-1',
          accountKey: 'server/user-a',
          actionKey: 'messages.send',
          payloadJson: '{}',
        ),
        optimisticRow: const KalamOptimisticRow(
          tableId: 'app.messages',
          rowKey: 'message-1',
          phase: KalamRowSyncPhase.pending,
          pendingValuesJson: '{"id":"message-1","text":"draft"}',
        ),
      );

      await messages.applyServerChange(
        const KalamChange(
          kind: KalamChangeKind.insert,
          rowKey: 'message-1',
          row: Message(id: 'message-1', text: 'authoritative'),
          seq: SeqId(12),
        ),
      );

      final rows = await messages.watchWithSyncState().first;
      final checkpoint = await store.readCheckpoint(
        accountKey: 'server/user-a',
        subscriptionId: 'conversation-1/messages',
      );
      expect(
        rows.single.value,
        const Message(id: 'message-1', text: 'authoritative'),
      );
      expect(rows.single.sync.phase, KalamRowSyncPhase.synced);
      expect(checkpoint?.seq, const SeqId(12));
    },
  );
}
