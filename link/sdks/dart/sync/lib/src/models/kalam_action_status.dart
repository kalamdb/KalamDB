/// Durable lifecycle of a queued action.
enum KalamActionStatus {
  queued,
  running,
  retryScheduled,
  succeeded,
  failed,
  cancelled,
}
