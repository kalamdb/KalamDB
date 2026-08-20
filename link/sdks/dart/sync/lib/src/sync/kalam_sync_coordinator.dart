import 'dart:async';

import '../actions/kalam_action_runner.dart';
import '../models/kalam_action_status.dart';
import '../models/kalam_action_record.dart';
import '../models/kalam_sync_state.dart';
import '../store/kalam_sync_store.dart';
import '../transport/kalam_link_transport.dart';
import '../transport/kalam_remote_batch.dart';
import '../transport/kalam_sync_transport.dart';
import 'kalam_consume_context.dart';
import 'kalam_event_consumer.dart';
import 'kalam_sync_subscription.dart';

/// Coordinates durable consumers, connection state, and outbox flushing.
final class KalamSyncCoordinator {
  KalamSyncCoordinator({
    required this.accountKey,
    required this.store,
    required this.transport,
    required this.actions,
    this.pauseIdleTimeout = const Duration(seconds: 5),
  }) {
    _connectionSubscription = transport.connectionStates
        .asyncMap(_handleConnection)
        .listen(null, onError: _onError);
    _actionSubscription = store.watchActions(accountKey).listen(_onActions);
  }

  final String accountKey;
  final KalamSyncStore store;
  final KalamSyncTransport transport;
  final KalamActionRunner actions;
  final Duration pauseIdleTimeout;
  final Map<String, _ActiveConsumer> _consumers = {};
  final StreamController<KalamSyncState> _states = StreamController.broadcast();
  StreamSubscription<void>? _connectionSubscription;
  StreamSubscription<List<KalamActionRecord>>? _actionSubscription;
  Timer? _retryTimer;
  KalamSyncState _state = const KalamSyncState();
  bool _connected = false;
  bool _paused = false;
  bool _disposed = false;
  bool _flushInProgress = false;
  bool _connectInProgress = false;
  int _inFlightApplies = 0;
  int _lifecycleGeneration = 0;

  KalamSyncState get state => _state;
  Stream<KalamSyncState> get states async* {
    yield _state;
    yield* _states.stream;
  }

  Future<KalamSyncSubscription> subscribe(KalamEventConsumer consumer) async {
    _ensureOpen();
    if (_consumers.containsKey(consumer.id)) {
      throw StateError('Consumer "${consumer.id}" is already subscribed.');
    }
    final active = _ActiveConsumer(consumer);
    _consumers[consumer.id] = active;
    if (!_paused) await _open(active);
    return KalamSyncSubscription(() async {
      _consumers.remove(consumer.id);
      active.cancelled = true;
      await active.subscription?.cancel();
    });
  }

  Future<void> pause() async {
    if (_paused || _disposed) return;
    final generation = ++_lifecycleGeneration;
    _paused = true;
    _emit(_state.copyWith(phase: KalamSyncPhase.paused, clearError: true));
    await _waitForIdle(generation);
    if (generation != _lifecycleGeneration || _disposed || !_paused) return;
    for (final active in _consumers.values) {
      await active.subscription?.cancel();
      active.subscription = null;
    }
    if (generation != _lifecycleGeneration || _disposed || !_paused) return;
    await transport.pause();
  }

  Future<void> resume() async {
    _ensureOpen();
    final generation = ++_lifecycleGeneration;
    if (!_paused) return;
    _paused = false;
    await transport.resume();
    if (generation != _lifecycleGeneration || _disposed || _paused) return;
    for (final active in _consumers.values) {
      await _open(active);
    }
    if (generation != _lifecycleGeneration || _disposed || _paused) return;
    _emit(
      _state.copyWith(
        phase: _connected ? KalamSyncPhase.catchingUp : KalamSyncPhase.offline,
        clearError: true,
      ),
    );
  }

  Future<void> dispose() async {
    if (_disposed) return;
    _disposed = true;
    _lifecycleGeneration++;
    for (final active in _consumers.values) {
      await active.subscription?.cancel();
    }
    _consumers.clear();
    await _connectionSubscription?.cancel();
    await _actionSubscription?.cancel();
    _retryTimer?.cancel();
    await _states.close();
  }

  Future<void> _waitForIdle(int generation) async {
    final deadline = DateTime.now().add(pauseIdleTimeout);
    while ((_flushInProgress || _inFlightApplies > 0) &&
        DateTime.now().isBefore(deadline)) {
      if (generation != _lifecycleGeneration || _disposed || !_paused) return;
      await Future<void>.delayed(const Duration(milliseconds: 10));
    }
  }

  Future<void> _open(_ActiveConsumer active) async {
    if (active.subscription != null) return;
    final checkpoint = await store.readCheckpoint(
      accountKey: accountKey,
      subscriptionId: active.consumer.id,
    );
    if (_paused || active.isCancelled || _disposed) return;
    if (!_connected || _state.phase != KalamSyncPhase.live) {
      _emit(
        _state.copyWith(phase: KalamSyncPhase.catchingUp, clearError: true),
      );
    }
    final generation = ++active.generation;
    active.subscription = transport
        .subscribe(
          sql: active.consumer.sql,
          subscriptionId: active.consumer.id,
          from: checkpoint?.seq,
          batchSize: active.consumer.batchSize,
          params: active.consumer.params,
        )
        .asyncMap((batch) => _applyBatch(active.consumer, batch))
        .listen(
          (_) {
            if (_connected) {
              _emit(
                _state.copyWith(
                  phase: KalamSyncPhase.live,
                  lastSuccessfulSync: DateTime.now().toUtc(),
                  clearError: true,
                ),
              );
            }
          },
          onError: (Object error, StackTrace stackTrace) {
            if (active.generation == generation) active.subscription = null;
            unawaited(_handleSubscribeError(active, error, stackTrace));
          },
          onDone: () {
            if (active.generation == generation) active.subscription = null;
          },
          cancelOnError: true,
        );
  }

  Future<void> _handleSubscribeError(
    _ActiveConsumer active,
    Object error,
    StackTrace stackTrace,
  ) async {
    if (_disposed || active.isCancelled) return;
    if (error is KalamSubscriptionException &&
        error.isExpiredCursor &&
        !active.rebootstrapAttempted &&
        !_paused) {
      active.rebootstrapAttempted = true;
      await store.deleteCheckpoint(
        accountKey: accountKey,
        subscriptionId: active.consumer.id,
      );
      await _open(active);
      return;
    }
    _onError(error, stackTrace);
  }

  Future<void> _applyBatch(
    KalamEventConsumer consumer,
    KalamRemoteBatch batch,
  ) async {
    _inFlightApplies++;
    try {
      final checkpoint = batch.checkpoint;
      if (checkpoint == null) {
        await batch.acknowledge();
        return;
      }

      final committed = await store.readCheckpoint(
        accountKey: accountKey,
        subscriptionId: consumer.id,
      );
      final changes = batch.changes.toList(growable: false)
        ..sort((first, second) => first.seq.compareTo(second.seq));
      final context = KalamConsumeContext(actions: actions);
      await store.applyAndCheckpoint(
        accountKey: accountKey,
        subscriptionId: consumer.id,
        seq: checkpoint,
        apply: () async {
          var appliedThrough = committed?.seq;
          for (final change in changes) {
            if (appliedThrough != null && change.seq <= appliedThrough) continue;
            await consumer.handle(change, context);
            appliedThrough = change.seq;
          }
          return null;
        },
      );
      await batch.acknowledge();
    } finally {
      _inFlightApplies--;
    }
  }

  Future<void> _handleConnection(KalamTransportConnection connection) async {
    _connected = connection == KalamTransportConnection.connected;
    if (_paused) return;
    if (!_connected) {
      for (final active in _consumers.values) {
        await active.subscription?.cancel();
        active.subscription = null;
      }
      _emit(_state.copyWith(phase: KalamSyncPhase.offline));
      if (_consumers.isNotEmpty) {
        unawaited(_ensureConnected());
      }
      return;
    }
    for (final active in _consumers.values) {
      await _open(active);
    }
    await _flushActions();
  }

  Future<void> _flushActions() async {
    if (_flushInProgress || !_connected || _paused) return;
    _flushInProgress = true;
    _emit(_state.copyWith(phase: KalamSyncPhase.flushing, clearError: true));
    try {
      await actions.flush();
      if (_connected && !_paused) {
        _emit(
          _state.copyWith(
            phase: KalamSyncPhase.live,
            lastSuccessfulSync: DateTime.now().toUtc(),
            clearError: true,
          ),
        );
      }
    } catch (error) {
      _onError(error);
    } finally {
      _flushInProgress = false;
    }
  }

  void _onActions(List<KalamActionRecord> records) {
    var pending = 0;
    var failed = 0;
    for (final record in records) {
      final status = record.status;
      if (status == KalamActionStatus.failed) {
        failed++;
      } else if (!record.isTerminal) {
        pending++;
      }
    }
    _emit(_state.copyWith(pendingActions: pending, failedActions: failed));
    _scheduleRetry(records);
    if (pending > 0 && !_paused) {
      if (_connected) {
        _flushActions();
      } else {
        unawaited(_ensureConnected());
      }
    }
  }

  Future<void> _ensureConnected() async {
    if (_connectInProgress || _connected || _paused || _disposed) return;
    _connectInProgress = true;
    try {
      await transport.ensureConnected();
    } catch (error) {
      _onError(error);
    } finally {
      _connectInProgress = false;
    }
  }

  void _scheduleRetry(List<KalamActionRecord> records) {
    _retryTimer?.cancel();
    if (!_connected || _paused) return;
    DateTime? earliest;
    for (final record in records) {
      if (record.status != KalamActionStatus.retryScheduled ||
          record.nextAttemptAt == null) {
        continue;
      }
      if (earliest == null || record.nextAttemptAt!.isBefore(earliest)) {
        earliest = record.nextAttemptAt;
      }
    }
    if (earliest == null) return;
    final delay = earliest.difference(DateTime.now().toUtc());
    _retryTimer = Timer(
      delay.isNegative ? Duration.zero : delay,
      _flushActions,
    );
  }

  void _onError(Object error, [StackTrace? _]) {
    if (_disposed) return;
    final code = _errorCode(error);
    _emit(
      _state.copyWith(
        phase: KalamSyncPhase.error,
        error: error.toString(),
        errorCode: code,
      ),
    );
  }

  String? _errorCode(Object error) {
    if (error is KalamSubscriptionException) return error.code;
    final text = error.toString();
    if (text.contains('TOKEN_EXPIRED')) return 'TOKEN_EXPIRED';
    if (text.contains('UNAUTHORIZED')) return 'UNAUTHORIZED';
    return null;
  }

  void _emit(KalamSyncState next) {
    if (_disposed || next == _state) return;
    _state = next;
    _states.add(next);
  }

  void _ensureOpen() {
    if (_disposed) throw StateError('KalamSyncCoordinator is disposed.');
  }
}

final class _ActiveConsumer {
  _ActiveConsumer(this.consumer);

  final KalamEventConsumer consumer;
  StreamSubscription<dynamic>? subscription;
  int generation = 0;
  bool cancelled = false;
  bool rebootstrapAttempted = false;

  bool get isCancelled => cancelled;
}
