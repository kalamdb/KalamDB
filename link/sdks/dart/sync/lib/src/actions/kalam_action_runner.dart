import 'dart:convert';

import 'package:kalam_link/kalam_link.dart';

import '../models/kalam_action_draft.dart';
import '../models/kalam_action_record.dart';
import '../models/kalam_optimistic_row.dart';
import '../models/kalam_optimistic_mutation.dart';
import '../models/kalam_retry_policy.dart';
import '../store/kalam_sync_store.dart';
import 'kalam_action_context.dart';
import 'kalam_action_registry.dart';

typedef KalamActionClock = DateTime Function();

/// Enqueues typed actions and flushes the durable outbox in FIFO order.
final class KalamActionRunner {
  KalamActionRunner({
    required this.store,
    required this.registry,
    required this.accountKey,
    KalamActionClock? clock,
    Future<KalamClient> Function()? resolveClient,
  }) : _clock = clock ?? DateTime.now,
       _resolveClient = resolveClient;

  final KalamSyncStore store;
  final KalamActionRegistry registry;
  final String accountKey;
  final KalamActionClock _clock;
  final Future<KalamClient> Function()? _resolveClient;
  Future<int>? _activeFlush;

  Future<KalamActionRecord> enqueue({
    required String actionKey,
    required String actionId,
    required Object? payload,
    String? orderingKey,
    KalamOptimisticRow? optimisticRow,
    Future<void> Function()? applyOptimistic,
    KalamOptimisticMutation? optimistic,
  }) {
    if (optimistic != null &&
        (optimisticRow != null || applyOptimistic != null)) {
      throw ArgumentError(
        'Use optimistic or optimisticRow/applyOptimistic, not both.',
      );
    }
    final definition = registry[actionKey];
    return store.enqueue(
      KalamActionDraft(
        id: actionId,
        accountKey: accountKey,
        actionKey: actionKey,
        version: definition.version,
        payloadJson: jsonEncode(definition.encodePayload(payload)),
        orderingKey: orderingKey,
      ),
      optimisticRow: optimistic?.row ?? optimisticRow,
      applyOptimistic: optimistic?.apply ?? applyOptimistic,
    );
  }

  /// Flushes currently eligible work. Concurrent calls share the active flush.
  Future<int> flush() {
    return _activeFlush ??= _flush().whenComplete(() => _activeFlush = null);
  }

  Future<int> _flush() async {
    var completed = 0;
    await store.recoverRunningActions(_clock(), accountKey: accountKey);
    final inFlight = <String, Future<void>>{};

    Future<void> launch(KalamActionRecord action) {
      final key = action.orderingKey ?? '';
      final future = _execute(action).whenComplete(() {
        inFlight.remove(key);
        completed++;
      });
      inFlight[key] = future;
      return future;
    }

    while (true) {
      final action = await store.claimNextAction(
        _clock(),
        accountKey: accountKey,
        blockedOrderingKeys: inFlight.keys.toSet(),
      );
      if (action == null) {
        if (inFlight.isEmpty) return completed;
        await Future.any(inFlight.values);
        continue;
      }
      launch(action);
    }
  }

  Future<void> _execute(KalamActionRecord action) async {
    final definition = registry[action.actionKey];
    if (definition.version != action.version) {
      await store.failAction(
        action.id,
        'Action version ${action.version} is not registered.',
        _clock(),
      );
      return;
    }

    try {
      final decoded = jsonDecode(action.payloadJson);
      if (decoded is! Map<String, Object?>) {
        throw const FormatException('Action payload must be a JSON object.');
      }
      await definition.executePayload(
        KalamActionContext(
          actionId: action.id,
          accountKey: action.accountKey,
          attempt: action.attemptCount,
          store: store,
          resolveClient: _resolveClient,
        ),
        decoded,
      );
      await store.completeAction(action.id, _clock());
    } on KalamPermanentActionException catch (error) {
      await store.failAction(action.id, error.message, _clock());
    } catch (error) {
      if (action.attemptCount >= definition.retryPolicy.maxAttempts) {
        await store.failAction(action.id, error.toString(), _clock());
      } else {
        final retryAt = _clock().add(
          definition.retryPolicy.delayForAttempt(action.attemptCount),
        );
        await store.scheduleRetry(action.id, error.toString(), retryAt);
      }
    }
  }
}
