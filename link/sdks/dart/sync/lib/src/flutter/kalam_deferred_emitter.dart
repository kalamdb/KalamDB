import 'dart:async';

/// Holds a value briefly so a short transition does not reach the listener.
final class KalamDeferredEmitter<T> {
  KalamDeferredEmitter({required this.delay, required this.onEmit});

  final Duration delay;
  final void Function(T value) onEmit;

  Timer? _timer;
  T? _pending;

  bool get hasPending => _timer != null;

  void defer(T value) {
    _pending = value;
    _timer ??= Timer(delay, () {
      _timer = null;
      final pending = _pending;
      _pending = null;
      if (pending != null) onEmit(pending);
    });
  }

  void emitNow(T value) {
    _timer?.cancel();
    _timer = null;
    _pending = null;
    onEmit(value);
  }

  void dispose() {
    _timer?.cancel();
    _timer = null;
    _pending = null;
  }
}
