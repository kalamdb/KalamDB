import 'dart:convert';

import '../store/kalam_sync_store.dart';

/// Runtime values and durable named steps available to an action executor.
final class KalamActionContext {
  KalamActionContext({
    required this.actionId,
    required this.accountKey,
    required this.attempt,
    required KalamSyncStore store,
  }) : _store = store;

  final String actionId;
  final String accountKey;
  final int attempt;
  final KalamSyncStore _store;

  String get idempotencyKey => actionId;

  /// Runs a sub-operation at most once across retries and process restarts.
  Future<T> step<T>(
    String name, {
    required Future<T> Function(String idempotencyKey) run,
    required Object? Function(T value) encode,
    required T Function(Object? value) decode,
  }) async {
    final stored = await _store.readCompletedStep(actionId, name);
    if (stored != null) return decode(jsonDecode(stored));

    try {
      final result = await run('$actionId/$name');
      await _store.completeStep(actionId, name, jsonEncode(encode(result)));
      return result;
    } catch (error) {
      await _store.failStep(actionId, name, error.toString());
      rethrow;
    }
  }
}
