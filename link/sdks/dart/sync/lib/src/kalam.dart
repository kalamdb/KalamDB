import 'dart:math';

import 'package:kalam_link/kalam_link.dart';

import 'actions/kalam_action_definition.dart';
import 'actions/kalam_action_registry.dart';
import 'actions/kalam_action_runner.dart';
import 'actions/kalam_dml_action.dart';
import 'database/kalam_sync_database.dart';
import 'flutter/kalam_database_factory.dart';
import 'models/kalam_account_identity.dart';
import 'models/kalam_sync_state.dart';
import 'store/kalam_sync_store.dart';
import 'sync/kalam_event_consumer.dart';
import 'sync/kalam_sync_coordinator.dart';
import 'sync/kalam_sync_subscription.dart';
import 'tables/kalam_table_binding.dart';
import 'tables/kalam_table_spec.dart';
import 'transport/kalam_link_transport.dart';
import 'transport/kalam_sync_transport.dart';

typedef KalamTransportFactory = Future<KalamSyncTransport> Function();

/// One account-scoped local-first KalamDB session.
final class Kalam {
  Kalam._({
    required this.identity,
    required this.database,
    required this.transport,
    required this.store,
    required this.actions,
    required this.sync,
  });

  final KalamAccountIdentity identity;
  final KalamSyncDatabase database;
  final KalamSyncTransport transport;
  final KalamSyncStore store;
  final KalamActionRunner actions;
  final KalamSyncCoordinator sync;
  bool _disposed = false;

  KalamSyncState get syncState => sync.state;
  Stream<KalamSyncState> get syncStates => sync.states;

  static Future<void> ensureInitialized() => KalamClient.init();

  /// Opens local storage immediately. The shared socket remains lazy until a
  /// consumer subscribes or an action is queued, so opening Kalam does not
  /// block Flutter's first UI.
  static Future<Kalam> open({
    required String url,
    required String subject,
    AuthProvider authProvider = _anonymousAuth,
    String namespace = 'default',
    Iterable<KalamActionDefinition<dynamic>> actionDefinitions = const [],
    KalamDatabaseFactory databaseFactory = const KalamFlutterDatabaseFactory(),
    KalamTransportFactory? transportFactory,
  }) async {
    await ensureInitialized();
    final identity = KalamAccountIdentity(
      serverUrl: url,
      subject: subject,
      namespace: namespace,
    );
    final database = await databaseFactory.open(identity);
    try {
      final transport =
          await (transportFactory?.call() ??
              KalamLinkTransport.connect(url: url, authProvider: authProvider));
      return Kalam.fromComponents(
        identity: identity,
        database: database,
        transport: transport,
        actionDefinitions: actionDefinitions,
      );
    } catch (_) {
      await database.close();
      rethrow;
    }
  }

  /// Low-level constructor for tests and custom transports.
  ///
  /// Generic bidirectional DML is registered when [transport] is a
  /// [KalamLinkTransport], or when [dmlClient] is passed for a wrapper
  /// transport that still talks through `kalam_link`.
  factory Kalam.fromComponents({
    required KalamAccountIdentity identity,
    required KalamSyncDatabase database,
    required KalamSyncTransport transport,
    Iterable<KalamActionDefinition<dynamic>> actionDefinitions = const [],
    KalamClient? dmlClient,
  }) {
    final client =
        dmlClient ??
        (transport is KalamLinkTransport ? transport.client : null);
    final definitions = <KalamActionDefinition<dynamic>>[
      if (client != null) kalamDmlAction(client),
      ...actionDefinitions,
    ];
    final store = KalamSyncStore(database);
    final actions = KalamActionRunner(
      accountKey: identity.accountKey,
      store: store,
      registry: KalamActionRegistry.of(definitions),
    );
    final sync = KalamSyncCoordinator(
      accountKey: identity.accountKey,
      store: store,
      transport: transport,
      actions: actions,
    );
    return Kalam._(
      identity: identity,
      database: database,
      transport: transport,
      store: store,
      actions: actions,
      sync: sync,
    );
  }

  Future<KalamSyncSubscription> subscribe(KalamEventConsumer consumer) {
    return sync.subscribe(consumer);
  }

  KalamTableBinding<T> table<T>(KalamTableSpec<T> spec) {
    return KalamTableBinding(
      spec: spec,
      accountKey: identity.accountKey,
      store: store,
    );
  }

  Future<void> pause() => sync.pause();

  Future<void> resume() => sync.resume();

  Future<void> dispose() async {
    if (_disposed) return;
    _disposed = true;
    await sync.dispose();
    await transport.dispose();
    await database.close();
  }

  /// Generates a cryptographically random RFC 4122 version 4 identifier.
  static String id() {
    final random = Random.secure();
    final bytes = List<int>.generate(16, (_) => random.nextInt(256));
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    final hex = bytes
        .map((byte) => byte.toRadixString(16).padLeft(2, '0'))
        .join();
    return '${hex.substring(0, 8)}-${hex.substring(8, 12)}-'
        '${hex.substring(12, 16)}-${hex.substring(16, 20)}-'
        '${hex.substring(20)}';
  }
}

Future<Auth> _anonymousAuth() async => const Auth.none();
