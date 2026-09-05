import 'dart:async';
import 'dart:convert';

import 'package:drift/native.dart';
import 'package:kalam_sync/drift.dart' hide isNotNull, isNull;
import 'package:flutter_test/flutter_test.dart';
import 'package:kalam_sync/kalam_sync.dart';

final class SendMessage {
  const SendMessage(this.text);

  final String text;
}

void main() {
  late KalamSyncDatabase database;
  late KalamSyncStore store;
  final now = DateTime.utc(2026, 8, 16, 12);

  setUp(() {
    database = KalamSyncDatabase(NativeDatabase.memory());
    store = KalamSyncStore(database, clock: () => now);
  });

  tearDown(() => database.close());

  KalamActionDefinition<SendMessage> definition({
    required Future<void> Function(
      KalamActionContext context,
      SendMessage payload,
    )
    execute,
    KalamRetryPolicy retryPolicy = const KalamRetryPolicy(),
  }) {
    return KalamActionDefinition(
      key: 'messages.send',
      codec: KalamActionCodec(
        encode: (payload) => {'text': payload.text},
        decode: (json) => SendMessage(json['text']! as String),
      ),
      retryPolicy: retryPolicy,
      execute: execute,
    );
  }

  test('dispose drains started work and leaves the backlog queued', () async {
    final started = Completer<void>();
    final release = Completer<void>();
    var count = 0;
    final runner = KalamActionRunner(
      store: store,
      accountKey: 'account',
      registry: KalamActionRegistry([
        definition(
          execute: (_, __) async {
            count++;
            if (count == 8) started.complete();
            await release.future;
          },
        ),
      ]),
    );
    for (var i = 0; i < 20; i++) {
      await runner.enqueue(
        actionKey: 'messages.send',
        actionId: 'action-$i',
        orderingKey: 'chat-$i',
        payload: const SendMessage('hello'),
      );
    }
    final flushing = runner.flush();
    await started.future;
    var disposed = false;
    final disposing = runner.dispose().then((_) => disposed = true);
    await Future<void>.delayed(Duration.zero);
    expect(disposed, isFalse);
    expect(count, 8);
    release.complete();
    await disposing;
    await flushing;
    expect(count, 8);
    expect(await runner.flush(), 0);
    final rows = await database.select(database.kalamActions).get();
    expect(rows.where((a) => a.status == 'queued').length, 12);
  });

  test(
    'encodes typed payloads and keeps the action id as idempotency key',
    () async {
      late KalamActionContext receivedContext;
      late SendMessage receivedPayload;
      final runner = KalamActionRunner(
        store: store,
        accountKey: 'server/user-a',
        registry: KalamActionRegistry([
          definition(
            execute: (context, payload) async {
              receivedContext = context;
              receivedPayload = payload;
            },
          ),
        ]),
        clock: () => now,
      );

      final queued = await runner.enqueue(
        actionKey: 'messages.send',
        actionId: 'action-1',
        payload: const SendMessage('hello'),
      );
      await runner.flush();

      expect(jsonDecode(queued.payloadJson), {'text': 'hello'});
      expect(receivedPayload.text, 'hello');
      expect(receivedContext.idempotencyKey, 'action-1');
      expect(
        (await store.readAction('action-1'))?.status,
        KalamActionStatus.succeeded,
      );
    },
  );

  test('rejects duplicate action keys at registration', () {
    final first = definition(execute: (_, _) async {});

    expect(() => KalamActionRegistry([first, first]), throwsArgumentError);
  });

  test('flushes queued actions in FIFO order', () async {
    final calls = <String>[];
    final runner = KalamActionRunner(
      store: store,
      accountKey: 'server/user-a',
      registry: KalamActionRegistry([
        definition(execute: (_, payload) async => calls.add(payload.text)),
      ]),
      clock: () => now,
    );

    await runner.enqueue(
      actionKey: 'messages.send',
      actionId: 'z-first',
      orderingKey: 'conversation-1',
      payload: const SendMessage('first'),
    );
    await runner.enqueue(
      actionKey: 'messages.send',
      actionId: 'a-second',
      orderingKey: 'conversation-1',
      payload: const SendMessage('second'),
    );

    expect(await runner.flush(), 2);
    expect(calls, ['first', 'second']);
  });

  test('schedules exponential retry and eventually succeeds', () async {
    var attempts = 0;
    var current = now;
    final runner = KalamActionRunner(
      store: store,
      accountKey: 'server/user-a',
      registry: KalamActionRegistry([
        definition(
          retryPolicy: const KalamRetryPolicy(
            maxAttempts: 3,
            initialDelay: Duration(seconds: 2),
          ),
          execute: (_, _) async {
            attempts++;
            if (attempts == 1) throw StateError('offline');
          },
        ),
      ]),
      clock: () => current,
    );
    await runner.enqueue(
      actionKey: 'messages.send',
      actionId: 'action-1',
      payload: const SendMessage('hello'),
    );

    await runner.flush();
    var action = await store.readAction('action-1');
    expect(action?.status, KalamActionStatus.retryScheduled);
    expect(action?.attemptCount, 1);
    expect(action?.nextAttemptAt, now.add(const Duration(seconds: 2)));

    expect(await runner.flush(), 0);
    current = now.add(const Duration(seconds: 2));
    expect(await runner.flush(), 1);
    action = await store.readAction('action-1');
    expect(action?.status, KalamActionStatus.succeeded);
  });

  test('permanent errors are not retried', () async {
    final runner = KalamActionRunner(
      store: store,
      accountKey: 'server/user-a',
      registry: KalamActionRegistry([
        definition(
          execute: (_, _) async {
            throw const KalamPermanentActionException('invalid recipient');
          },
        ),
      ]),
      clock: () => now,
    );
    await runner.enqueue(
      actionKey: 'messages.send',
      actionId: 'action-1',
      payload: const SendMessage('hello'),
    );

    await runner.flush();

    final action = await store.readAction('action-1');
    expect(action?.status, KalamActionStatus.failed);
    expect(action?.lastError, contains('invalid recipient'));
  });

  test('recovers a running action after restart', () async {
    await store.enqueue(
      const KalamActionDraft(
        id: 'action-1',
        accountKey: 'server/user-a',
        actionKey: 'messages.send',
        payloadJson: '{"text":"hello"}',
      ),
    );
    await store.claimNextAction(now);

    var calls = 0;
    final restarted = KalamActionRunner(
      store: store,
      accountKey: 'server/user-a',
      registry: KalamActionRegistry([
        definition(execute: (_, _) async => calls++),
      ]),
      clock: () => now,
    );

    expect(await restarted.flush(), 1);
    expect(calls, 1);
    expect(
      (await store.readAction('action-1'))?.status,
      KalamActionStatus.succeeded,
    );
  });

  test('completed named steps are reused when an action retries', () async {
    var actionAttempts = 0;
    var stepCalls = 0;
    var current = now;
    final runner = KalamActionRunner(
      store: store,
      accountKey: 'server/user-a',
      registry: KalamActionRegistry([
        definition(
          retryPolicy: const KalamRetryPolicy(
            maxAttempts: 2,
            initialDelay: Duration(seconds: 1),
          ),
          execute: (context, _) async {
            actionAttempts++;
            final value = await context.step<int>(
              'upload',
              run: (stepIdempotencyKey) async {
                expect(stepIdempotencyKey, 'action-1/upload');
                stepCalls++;
                return 42;
              },
              encode: (value) => value,
              decode: (value) => value as int,
            );
            expect(value, 42);
            if (actionAttempts == 1) throw StateError('send failed');
          },
        ),
      ]),
      clock: () => current,
    );
    await runner.enqueue(
      actionKey: 'messages.send',
      actionId: 'action-1',
      payload: const SendMessage('hello'),
    );

    await runner.flush();
    current = now.add(const Duration(seconds: 1));
    await runner.flush();

    expect(actionAttempts, 2);
    expect(stepCalls, 1);
  });

  test('server echo wins when the action response is lost', () async {
    late KalamActionRunner runner;
    runner = KalamActionRunner(
      store: store,
      accountKey: 'server/user-a',
      registry: KalamActionRegistry([
        definition(
          execute: (_, _) async {
            await store.markRowSynced(
              accountKey: 'server/user-a',
              tableId: 'app.messages',
              rowKey: 'message-1',
              seq: const SeqId(15),
            );
            throw StateError('response lost after commit');
          },
        ),
      ]),
      clock: () => now,
    );
    await runner.enqueue(
      actionKey: 'messages.send',
      actionId: 'action-1',
      payload: const SendMessage('hello'),
      optimisticRow: const KalamOptimisticRow(
        tableId: 'app.messages',
        rowKey: 'message-1',
        phase: KalamRowSyncPhase.pending,
      ),
    );

    await runner.flush();

    expect(
      (await store.readAction('action-1'))?.status,
      KalamActionStatus.succeeded,
    );
  });

  test('unrelated ordering keys flush concurrently', () async {
    final firstStarted = Completer<void>();
    final releaseFirst = Completer<void>();
    final secondStarted = Completer<void>();
    final runner = KalamActionRunner(
      store: store,
      accountKey: 'server/user-a',
      registry: KalamActionRegistry([
        definition(
          execute: (_, payload) async {
            if (payload.text == 'first') {
              firstStarted.complete();
              await releaseFirst.future;
            } else {
              secondStarted.complete();
            }
          },
        ),
      ]),
      clock: () => now,
    );

    await runner.enqueue(
      actionKey: 'messages.send',
      actionId: 'a',
      orderingKey: 'conversation-1',
      payload: const SendMessage('first'),
    );
    await runner.enqueue(
      actionKey: 'messages.send',
      actionId: 'b',
      orderingKey: 'conversation-2',
      payload: const SendMessage('second'),
    );

    final flushing = runner.flush();
    await firstStarted.future;
    await secondStarted.future;
    releaseFirst.complete();
    expect(await flushing, 2);
  });
}
