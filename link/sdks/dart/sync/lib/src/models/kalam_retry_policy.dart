/// Retry limits and exponential backoff for a durable action.
final class KalamRetryPolicy {
  const KalamRetryPolicy({
    this.maxAttempts = 5,
    this.initialDelay = const Duration(seconds: 1),
    this.maxDelay = const Duration(minutes: 5),
    this.multiplier = 2,
  }) : assert(maxAttempts > 0),
       assert(multiplier >= 1);

  final int maxAttempts;
  final Duration initialDelay;
  final Duration maxDelay;
  final int multiplier;

  Duration delayForAttempt(int attempt) {
    var milliseconds = initialDelay.inMilliseconds;
    for (var index = 1; index < attempt; index++) {
      milliseconds *= multiplier;
      if (milliseconds >= maxDelay.inMilliseconds) return maxDelay;
    }
    return Duration(
      milliseconds: milliseconds.clamp(0, maxDelay.inMilliseconds),
    );
  }
}

/// Marks an action error as non-retryable.
final class KalamPermanentActionException implements Exception {
  const KalamPermanentActionException(this.message);

  final String message;

  @override
  String toString() => 'KalamPermanentActionException: $message';
}
