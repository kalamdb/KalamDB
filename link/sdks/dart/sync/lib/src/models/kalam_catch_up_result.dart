/// Outcome of a bounded, isolate-safe HTTP catch-up pass.
final class KalamCatchUpResult {
  const KalamCatchUpResult({
    required this.appliedCount,
    required this.hasMore,
    required this.timedOut,
    this.appliedByConsumer = const {},
  });

  final int appliedCount;
  final bool hasMore;
  final bool timedOut;
  final Map<String, int> appliedByConsumer;
}
