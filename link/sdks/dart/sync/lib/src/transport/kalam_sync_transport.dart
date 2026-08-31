import 'package:kalam_link/kalam_link.dart';

import 'kalam_remote_batch.dart';

enum KalamTransportConnection { disconnected, connected }

/// Small seam that keeps synchronization independent from the socket client.
abstract interface class KalamSyncTransport {
  Stream<KalamTransportConnection> get connectionStates;

  Stream<KalamRemoteBatch> subscribe({
    required String sql,
    required String subscriptionId,
    SeqId? from,
    int? batchSize,
    List<Object?>? params,
  });

  Future<void> ensureConnected();

  Future<void> pause();

  Future<void> resume();

  Future<void> dispose();
}
