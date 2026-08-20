import 'dart:async';

import 'package:kalam_link/kalam_link.dart';

import '../models/kalam_change.dart';
import 'kalam_remote_batch.dart';
import 'kalam_remote_change.dart';
import 'kalam_sync_transport.dart';

/// Adapts the existing shared `kalam_link` socket to the sync engine.
final class KalamLinkTransport implements KalamSyncTransport {
  KalamLinkTransport._({
    required String url,
    required AuthProvider authProvider,
    required Duration? keepaliveInterval,
    KalamClient? client,
    required StreamController<KalamTransportConnection> connections,
    required KalamTransportConnection connection,
  }) : _url = url,
       _authProvider = authProvider,
       _keepaliveInterval = keepaliveInterval,
       _client = client,
       _connections = connections,
       _connection = connection;

  final String _url;
  final AuthProvider _authProvider;
  final Duration? _keepaliveInterval;
  KalamClient? _client;
  Future<KalamClient>? _connecting;
  final StreamController<KalamTransportConnection> _connections;
  KalamTransportConnection _connection;
  var _disposed = false;
  var _connectCount = 0;

  /// Opens SQLite-side transport metadata without resolving auth or connecting.
  factory KalamLinkTransport.deferred({
    required String url,
    AuthProvider authProvider = _anonymousAuth,
    Duration? keepaliveInterval,
  }) {
    return KalamLinkTransport._(
      url: url,
      authProvider: authProvider,
      keepaliveInterval: keepaliveInterval,
      connections: StreamController<KalamTransportConnection>.broadcast(),
      connection: KalamTransportConnection.disconnected,
    );
  }

  static Future<KalamLinkTransport> connect({
    required String url,
    AuthProvider authProvider = _anonymousAuth,
    Duration? keepaliveInterval,
  }) async {
    final transport = KalamLinkTransport.deferred(
      url: url,
      authProvider: authProvider,
      keepaliveInterval: keepaliveInterval,
    );
    await transport.ensureClient();
    return transport;
  }

  KalamClient? get clientOrNull => _client;

  KalamClient get client {
    final client = _client;
    if (client == null) {
      throw StateError('KalamLinkTransport has not connected yet.');
    }
    return client;
  }

  int get connectCount => _connectCount;

  Future<KalamClient> ensureClient() {
    if (_disposed) {
      throw StateError('KalamLinkTransport is disposed.');
    }
    final existing = _client;
    if (existing != null) return Future.value(existing);
    return _connecting ??= _connectClient().whenComplete(() {
      _connecting = null;
    });
  }

  Future<KalamClient> _connectClient() async {
    _connectCount++;
    final connections = _connections;
    var current = _connection;
    void Function(KalamTransportConnection connection) emit = (connection) {
      current = connection;
      if (!connections.isClosed) connections.add(connection);
    };
    final client = await KalamClient.connect(
      url: _url,
      authProvider: _authProvider,
      wsLazyConnect: true,
      keepaliveInterval: _keepaliveInterval,
      connectionHandlers: ConnectionHandlers(
        onConnect: () => emit(KalamTransportConnection.connected),
        onDisconnect: (_) => emit(KalamTransportConnection.disconnected),
        onError: (_) => emit(KalamTransportConnection.disconnected),
      ),
    );
    _client = client;
    _connection = current;
    emit = _emitConnection;
    return client;
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
    List<Object?>? params,
  }) async* {
    final client = await ensureClient();
    yield* client
        .liveEventsWithAck(
          sql,
          subscriptionId: subscriptionId,
          from: from,
          batchSize: batchSize,
          params: params,
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
    final client = await ensureClient();
    if (!await client.isConnected) await client.reconnectWebSocket();
    if (await client.isConnected) {
      _emitConnection(KalamTransportConnection.connected);
    }
  }

  @override
  Future<void> pause() async {
    final client = _client;
    if (client == null) {
      _emitConnection(KalamTransportConnection.disconnected);
      return;
    }
    await client.disconnectWebSocket();
    _emitConnection(KalamTransportConnection.disconnected);
  }

  @override
  Future<void> resume() async {
    final client = await ensureClient();
    await client.reconnectWebSocket();
    if (await client.isConnected) {
      _emitConnection(KalamTransportConnection.connected);
    }
  }

  @override
  Future<void> dispose() async {
    _disposed = true;
    await _client?.dispose();
    _client = null;
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

  bool get isExpiredCursor {
    final haystack = '$code $message'.toLowerCase();
    return haystack.contains('expired') || haystack.contains('stale');
  }

  @override
  String toString() => 'KalamSubscriptionException($code): $message';
}

Future<Auth> _anonymousAuth() async => const Auth.none();
