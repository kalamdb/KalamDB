/// Controls how local writes are handled for a synchronized table.
enum KalamSyncMode {
  /// Local writes are applied immediately and queued for KalamDB.
  bidirectional,

  /// Backend rows are authoritative; local intent uses custom action overlays.
  replicaOnly,
}
