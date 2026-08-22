import 'dart:convert';

import 'package:kalam_link/kalam_link.dart';

import '../store/kalam_sync_store.dart';

/// Runtime values and durable named steps available to an action executor.
final class KalamActionContext {
  KalamActionContext({
    required this.actionId,
    required this.accountKey,
    required this.attempt,
    required KalamSyncStore store,
    Future<KalamClient> Function()? resolveClient,
  }) : _store = store,
       _resolveClient = resolveClient;

  final String actionId;
  final String accountKey;
  final int attempt;
  final KalamSyncStore _store;
  final Future<KalamClient> Function()? _resolveClient;

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

  /// Uploads `FILE("placeholder")` parts at most once per [stepName].
  ///
  /// Persist the step result, not the file bytes. Retrying an action reuses a
  /// completed upload instead of sending the multipart body again.
  Future<QueryResponse> queryWithFiles(
    String stepName, {
    required String sql,
    required List<KalamFileUpload> files,
    List<dynamic>? params,
    String? namespace,
  }) {
    return step<Map<String, Object?>>(
      stepName,
      run: (_) async {
        final client = await requireClient();
        final response = await client.queryWithFiles(
          sql,
          files: files,
          params: params,
          namespace: namespace,
        );
        if (!response.success) {
          throw StateError(
            response.error?.toString() ?? 'FILE query was rejected.',
          );
        }
        return {
          'success': response.success,
          'tookMs': response.tookMs,
        };
      },
      encode: (value) => value,
      decode: (value) => Map<String, Object?>.from(value! as Map),
    ).then(
      (encoded) => QueryResponse(
        success: encoded['success'] == true,
        results: const [],
        tookMs: (encoded['tookMs'] as num?)?.toDouble(),
      ),
    );
  }

  Future<KalamClient> requireClient() {
    final resolve = _resolveClient;
    if (resolve == null) {
      throw StateError(
        'This action context has no KalamClient. Pass a client through '
        'Kalam.open / Kalam.fromComponents to use queryWithFiles.',
      );
    }
    return resolve();
  }
}
