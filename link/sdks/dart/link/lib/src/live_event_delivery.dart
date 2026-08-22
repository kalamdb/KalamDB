import 'dart:async';

import 'models.dart';

/// One live event whose reconnect progress advances only after acknowledgement.
final class LiveEventDelivery {
  LiveEventDelivery({
    required this.event,
    required this.checkpoint,
    required Future<void> Function() acknowledge,
  }) : _acknowledge = acknowledge;

  final ChangeEvent event;
  final LiveCheckpoint? checkpoint;
  final Future<void> Function() _acknowledge;
  final Completer<void> _progress = Completer<void>();
  Future<void>? _acknowledgement;

  /// Acknowledge this event after its durable local transaction commits.
  /// Repeated calls share the same acknowledgement.
  Future<void> acknowledge() {
    final existing = _acknowledgement;
    if (existing != null) return existing;
    final acknowledgement = Future<void>.sync(_acknowledge);
    _acknowledgement = acknowledgement;
    acknowledgement.then(
      (_) => _progress.complete(),
      onError: (Object error, StackTrace stackTrace) {
        _progress.completeError(error, stackTrace);
      },
    );
    return acknowledgement;
  }

  /// Completes when acknowledgement succeeds and fails when it fails.
  Future<void> get whenAcknowledged =>
      checkpoint == null ? Future.value() : _progress.future;
}
