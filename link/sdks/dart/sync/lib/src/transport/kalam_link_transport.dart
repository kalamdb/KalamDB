import 'dart:async';

import 'package:kalam_link/kalam_link.dart';

import '../models/kalam_change.dart';
import 'kalam_remote_batch.dart';
import 'kalam_remote_change.dart';
import 'kalam_sync_transport.dart';

/// Adapts the existing shared `kalam_link` socket to the sync engine.
final class KalamLinkTransport implements KalamSyncTransport {
  KalamLinkTransport._(this.client, this._connections, this._connection);

  final KalamClient client;
  final StreamController<KalamTransportConnection> _connections;
  KalamTransportConnection _connection;

  static Future<KalamLinkTransport> connect({
    required String url,
    AuthProvider authProvider = _anonymousAuth,
  }) async {
    final connections = StreamController<KalamTransportConnection>.broadcast();
    var current = KalamTransportConnection.disconnected;
    void Function(KalamTransportConnection connection) emit = (connection) {
      current = connection;
      connections.add(connection);
    };
    final client = await KalamClient.connect(
      url: url,
      authProvider: authProvider,
      wsLazyConnect: true,
      connectionHandlers: ConnectionHandlers(
        onConnect: () => emit(KalamTransportConnection.connected),
        onDisconnect: (_) => emit(KalamTransportConnection.disconnected),
        onError: (_) => emit(KalamTransportConnection.disconnected),
      ),
    );
    final transport = KalamLinkTransport._(client, connections, current);
    emit = transport._emitConnection;
    return transport;
  }

  @override
  Stream<KalamTransportConnection> get connectionStates async* {
    yield _connection;
    yield* _connections.stream;
  }

  @override
  Stream<KalamRemoteBatch> subscribe({
    required String sql,
    required String subscriptionId,
    SeqId? from,
    int? batchSize,
  }) {
    return client
        .liveEventsWithAck(
          sql,
          subscriptionId: subscriptionId,
          from: from,
          batchSize: batchSize,
        )
        .map((delivery) {
          _emitConnection(KalamTransportConnection.connected);
          return KalamRemoteBatch(
            changes: decodeEvent(delivery.event),
            checkpoint: delivery.checkpoint?.lastSeqId,
            acknowledge: delivery.acknowledge,
          );
        });
  }

  @override
  Future<void> ensureConnected() async {
    if (!await client.isConnected) await client.reconnectWebSocket();
    if (await client.isConnected) {
      _emitConnection(KalamTransportConnection.connected);
    }
  }

  @override
  Future<void> pause() async {
    await client.disconnectWebSocket();
    _emitConnection(KalamTransportConnection.disconnected);
  }

  @override
  Future<void> resume() async {
    await client.reconnectWebSocket();
    if (await client.isConnected) {
      _emitConnection(KalamTransportConnection.connected);
    }
  }

  @override
  Future<void> dispose() async {
    await client.dispose();
    await _connections.close();
  }

  void _emitConnection(KalamTransportConnection connection) {
    if (_connection == connection || _connections.isClosed) return;
    _connection = connection;
    _connections.add(connection);
  }

  /// Converts one public kalam_link event into ordered, checkpointable rows.
  static List<KalamRemoteChange> decodeEvent(ChangeEvent event) {
    final changes = switch (event) {
      AckEvent() => <KalamRemoteChange>[],
      SubscriptionError(:final code, :final message) =>
        throw KalamSubscriptionException(code, message),
      InitialDataBatch(:final rows) => [
        for (final row in rows)
          _change(KalamChangeKind.insert, row, initial: true),
      ],
      InsertEvent(:final rows) => [
        for (final row in rows) _change(KalamChangeKind.insert, row),
      ],
      UpdateEvent(:final rows, :final oldRows) => [
        for (var index = 0; index < rows.length; index++)
          _change(
            KalamChangeKind.update,
            rows[index],
            oldRow: index < oldRows.length ? oldRows[index] : null,
          ),
      ],
      DeleteEvent(:final oldRows) => [
        for (final row in oldRows) _change(KalamChangeKind.delete, row),
      ],
    };
    changes.sort((first, second) => first.seq.compareTo(second.seq));
    return changes;
  }

  static KalamRemoteChange _change(
    KalamChangeKind kind,
    Map<String, KalamCellValue> values, {
    Map<String, KalamCellValue>? oldRow,
    bool initial = false,
  }) {
    final seq = values['_seq']?.asSeqId() ?? oldRow?['_seq']?.asSeqId();
    if (seq == null) {
      throw const FormatException(
        'A synchronized KalamDB row must include its _seq value.',
      );
    }
    return KalamRemoteChange(
      kind: kind,
      seq: seq,
      row: _plainRow(values),
      oldRow: oldRow == null ? null : _plainRow(oldRow),
      initial: initial,
    );
  }

  static Map<String, Object?> _plainRow(Map<String, KalamCellValue> values) {
    return {
      for (final entry in values.entries)
        if (entry.key != '_seq') entry.key: entry.value.toJson(),
    };
  }
}

final class KalamSubscriptionException implements Exception {
  const KalamSubscriptionException(this.code, this.message);

  final String code;
  final String message;

  @override
  String toString() => 'KalamSubscriptionException($code): $message';
}

Future<Auth> _anonymousAuth() async => const Auth.none();
