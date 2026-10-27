/// One in-flight token refresh, then pause and resume of the same session.
final class KalamAuthRecovery {
  var _inFlight = false;

  Future<void> recover({
    required bool enabled,
    required Future<void> Function() refresh,
    required Future<void> Function() pause,
    required Future<void> Function() resume,
  }) async {
    if (_inFlight || !enabled) return;
    _inFlight = true;
    try {
      await refresh();
      await pause();
      await resume();
    } finally {
      _inFlight = false;
    }
  }
}
