import 'dart:async';

import 'package:kalam_link/kalam_link.dart';

import 'kalam_remote_change.dart';

/// One server delivery that is acknowledged only after durable local apply.
final class KalamRemoteBatch {
  KalamRemoteBatch({
    required this.changes,
    required this.checkpoint,
    required Future<void> Function() acknowledge,
    this.snapshotComplete = false,
  }) : _acknowledge = acknowledge;

  final List<KalamRemoteChange> changes;
  final VersionId? checkpoint;

  /// True after the server has delivered the final initial snapshot batch.
  final bool snapshotComplete;
  final Future<void> Function() _acknowledge;
  Future<void>? _acknowledgement;

  /// Advances transport resume progress. Calls are idempotent.
  Future<void> acknowledge() => _acknowledgement ??= _acknowledge();
}
