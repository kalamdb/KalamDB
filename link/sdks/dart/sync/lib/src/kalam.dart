import 'dart:async';
import 'dart:math';

import 'package:kalam_link/kalam_link.dart';

import 'actions/kalam_action_definition.dart';
import 'actions/kalam_action_registry.dart';
import 'actions/kalam_action_runner.dart';
import 'actions/kalam_dml_action.dart';
import 'database/kalam_sync_database.dart';
import 'flutter/kalam_database_factory.dart';
import 'models/kalam_account_identity.dart';
import 'models/kalam_catch_up_result.dart';
import 'models/kalam_sync_state.dart';
import 'store/kalam_sync_store.dart';
import 'sync/kalam_consume_context.dart';
import 'sync/kalam_event_consumer.dart';
import 'sync/kalam_sync_coordinator.dart';
import 'sync/kalam_sync_subscription.dart';
import 'tables/kalam_table_binding.dart';
import 'tables/kalam_table_spec.dart';
import 'transport/kalam_link_transport.dart';
import 'transport/kalam_remote_batch.dart';
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

  /// Opens local storage immediately. Auth and the shared socket stay lazy
  /// until a consumer subscribes or an action is queued.
  static Future<Kalam> open({
    required String url,
    required String subject,
    AuthProvider authProvider = _anonymousAuth,
    String namespace = 'default',
    Iterable<KalamActionDefinition<dynamic>> actionDefinitions = const [],
    KalamDatabaseFactory databaseFactory = const KalamFlutterDatabaseFactory(),
    KalamTransportFactory? transportFactory,
    Duration? keepaliveInterval,
  }) async {
    await ensureInitialized();
    final identity = KalamAccountIdentity(
      serverUrl: url,
      subject: subject,
      namespace: namespace,
    );
    final database = await databaseFactory.open(identity);
    try {
      final transport = transportFactory != null
          ? await transportFactory()
          : KalamLinkTransport.deferred(
              url: url,
              authProvider: authProvider,
              keepaliveInterval: keepaliveInterval,
            );
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

  /// Bounded catch-up for headless isolates using the same live query as
  /// foreground subscribe (`from:` + `batchSize`), then disconnects.
  static Future<KalamCatchUpResult> catchUp({
    required String url,
    required String subject,
    AuthProvider authProvider = _anonymousAuth,
    String namespace = 'default',
    required Iterable<KalamEventConsumer> consumers,
    Iterable<KalamActionDefinition<dynamic>> actionDefinitions = const [],
    int rowLimit = 100,
    Duration timeout = const Duration(seconds: 30),
    bool flushOutbox = false,
    KalamDatabaseFactory databaseFactory = const KalamFlutterDatabaseFactory(),
    Duration? keepaliveInterval,
    KalamSyncTransport? transport,
    KalamTransportFactory? transportFactory,
    KalamSyncDatabase? database,
  }) async {
    await ensureInitialized();
    final identity = KalamAccountIdentity(
      serverUrl: url,
      subject: subject,
      namespace: namespace,
    );
    final ownedDatabase = database ?? await databaseFactory.open(identity);
    final ownsDatabase = database == null;
    final ownsTransport = transport == null && transportFactory == null;
    final activeTransport =
        transport ??
        (transportFactory != null
            ? await transportFactory()
            : KalamLinkTransport.deferred(
                url: url,
                authProvider: authProvider,
                keepaliveInterval: keepaliveInterval,
              ));
    try {
      final store = KalamSyncStore(ownedDatabase);
      final resolveClient = activeTransport is KalamLinkTransport
          ? activeTransport.ensureClient
          : null;
      final actions = KalamActionRunner(
        accountKey: identity.accountKey,
        store: store,
        registry: KalamActionRegistry.of(actionDefinitions),
        resolveClient: resolveClient,
      );
      var appliedCount = 0;
      var hasMore = false;
      var timedOut = false;
      final appliedByConsumer = <String, int>{};
      final deadline = DateTime.now().add(timeout);
      final context = KalamConsumeContext(actions: actions);

      for (final consumer in consumers) {
        if (DateTime.now().isAfter(deadline)) {
          timedOut = true;
          break;
        }
        final result = await _catchUpConsumer(
          accountKey: identity.accountKey,
          store: store,
          transport: activeTransport,
          consumer: consumer,
          context: context,
          rowLimit: rowLimit,
          deadline: deadline,
        );
        appliedByConsumer[consumer.id] = result.applied;
        appliedCount += result.applied;
        if (result.hasMore) hasMore = true;
        if (result.timedOut) timedOut = true;
      }

      if (flushOutbox) {
        await actions.flush();
      }

      return KalamCatchUpResult(
        appliedCount: appliedCount,
        hasMore: hasMore,
        timedOut: timedOut,
        appliedByConsumer: appliedByConsumer,
      );
    } finally {
      if (ownsTransport) await activeTransport.dispose();
      if (ownsDatabase) await ownedDatabase.close();
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
    Future<KalamClient> Function()? resolveClient;
    if (dmlClient != null) {
      resolveClient = () async => dmlClient;
    } else if (transport is KalamLinkTransport) {
      resolveClient = transport.ensureClient;
    }
    final definitions = <KalamActionDefinition<dynamic>>[
      if (resolveClient != null) kalamDmlAction(resolveClient),
      ...actionDefinitions,
    ];
    final store = KalamSyncStore(database);
    final actions = KalamActionRunner(
      accountKey: identity.accountKey,
      store: store,
      registry: KalamActionRegistry.of(definitions),
      resolveClient: resolveClient,
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

  KalamTableBinding<T> table<T>(
    KalamTableSpec<T> spec, {
    KalamRowWatch<T>? watchLocal,
    KalamRowUpsert<T>? upsertLocal,
    KalamRowDelete? deleteLocal,
  }) {
    return KalamTableBinding(
      spec: spec,
      accountKey: identity.accountKey,
      store: store,
      watchLocal: watchLocal,
      upsertLocal: upsertLocal,
      deleteLocal: deleteLocal,
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

final class _CatchUpConsumerResult {
  const _CatchUpConsumerResult({
    required this.applied,
    required this.hasMore,
    required this.timedOut,
  });

  final int applied;
  final bool hasMore;
  final bool timedOut;
}

Future<_CatchUpConsumerResult> _catchUpConsumer({
  required String accountKey,
  required KalamSyncStore store,
  required KalamSyncTransport transport,
  required KalamEventConsumer consumer,
  required KalamConsumeContext context,
  required int rowLimit,
  required DateTime deadline,
}) async {
  final checkpoint = await store.readCheckpoint(
    accountKey: accountKey,
    subscriptionId: consumer.id,
  );
  final remaining = deadline.difference(DateTime.now());
  if (remaining.isNegative) {
    return const _CatchUpConsumerResult(
      applied: 0,
      hasMore: false,
      timedOut: true,
    );
  }

  final batchSize = min(consumer.batchSize ?? rowLimit, rowLimit);
  var applied = 0;
  var hasMore = false;
  var timedOut = false;
  final done = Completer<void>();
  late final StreamSubscription<KalamRemoteBatch> subscription;

  Future<void> stop() async {
    await subscription.cancel();
    if (!done.isCompleted) done.complete();
  }

  subscription = transport
      .subscribe(
        sql: consumer.sql,
        subscriptionId: consumer.id,
        from: checkpoint?.seq,
        batchSize: batchSize,
        params: consumer.params,
      )
      .listen(
        (batch) async {
          subscription.pause();
          try {
            final committed = await store.readCheckpoint(
              accountKey: accountKey,
              subscriptionId: consumer.id,
            );
            final changes = batch.changes.toList(growable: false)
              ..sort((first, second) => first.seq.compareTo(second.seq));
            final checkpointSeq = batch.checkpoint;
            if (checkpointSeq != null) {
              var count = 0;
              await store.applyAndCheckpoint(
                accountKey: accountKey,
                subscriptionId: consumer.id,
                seq: checkpointSeq,
                apply: () async {
                  var appliedThrough = committed?.seq;
                  for (final change in changes) {
                    if (appliedThrough != null &&
                        change.seq <= appliedThrough) {
                      continue;
                    }
                    await consumer.handle(change, context);
                    appliedThrough = change.seq;
                    count++;
                  }
                  return null;
                },
              );
              applied += count;
              await batch.acknowledge();
              if (applied >= rowLimit) {
                hasMore = true;
                await stop();
                return;
              }
              if (count < batchSize) {
                await stop();
                return;
              }
            } else {
              await batch.acknowledge();
            }
          } catch (error, stackTrace) {
            if (!done.isCompleted) done.completeError(error, stackTrace);
            await subscription.cancel();
            return;
          }
          if (!done.isCompleted) subscription.resume();
        },
        onError: (Object error, StackTrace stackTrace) {
          if (!done.isCompleted) done.completeError(error, stackTrace);
        },
        onDone: () {
          if (!done.isCompleted) done.complete();
        },
        cancelOnError: true,
      );

  try {
    await done.future.timeout(remaining);
  } on TimeoutException {
    timedOut = true;
    await subscription.cancel();
  }

  return _CatchUpConsumerResult(
    applied: applied,
    hasMore: hasMore,
    timedOut: timedOut,
  );
}
