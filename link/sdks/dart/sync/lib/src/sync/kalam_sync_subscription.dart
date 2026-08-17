import 'dart:async';

/// Handle returned for a widget- or service-scoped sync consumer.
final class KalamSyncSubscription {
  KalamSyncSubscription(FutureOr<void> Function() cancel) : _cancel = cancel;

  final FutureOr<void> Function() _cancel;
  bool _isCancelled = false;

  bool get isCancelled => _isCancelled;

  Future<void> cancel() async {
    if (_isCancelled) return;
    _isCancelled = true;
    await _cancel();
  }
}
