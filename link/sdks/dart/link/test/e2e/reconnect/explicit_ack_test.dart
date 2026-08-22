import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:kalam_link/kalam_link.dart';

import '../helpers.dart';

void main() {
  test(
    'explicit acknowledgement advances resume only after durable consumer work',
    () async {
      await ensureSdkReady();
      final client = await connectJwtClient();
      final table = 'default.${uniqueName('dart_explicit_ack')}';
      final subscriptionId = uniqueName('explicit-ack');
      addTearDown(() async {
        await dropTable(client, table);
        await _safeDispose(client);
      });
      await client.query(
        'CREATE TABLE $table (id BIGINT PRIMARY KEY, value TEXT NOT NULL)',
      );

      final events = client.liveEventsWithAck(
        'SELECT * FROM $table',
        subscriptionId: subscriptionId,
      );
      final ready = Completer<void>();
      final inserted = Completer<LiveEventDelivery>();
      var activeSubscriptionId = subscriptionId;
      final subscription = events.listen((delivery) {
        if (delivery.event case AckEvent(:final subscriptionId)) {
          activeSubscriptionId = subscriptionId;
          if (!ready.isCompleted) ready.complete();
        }
        if (delivery.event is InsertEvent && !inserted.isCompleted) {
          inserted.complete(delivery);
        }
      });
      addTearDown(() => _safeCancel(subscription));
      await ready.future.timeout(const Duration(seconds: 10));
      await client.query(
        "INSERT INTO $table (id, value) VALUES (1, 'one')",
      );

      final delivery =
          await inserted.future.timeout(const Duration(seconds: 10));
      final seq =
          (delivery.event as InsertEvent).rows.single['_seq']!.asSeqId()!;
      expect(
        (await _subscription(client, activeSubscriptionId)).lastSeqId,
        isNot(seq),
        reason: 'delivery must not move the reconnect cursor',
      );

      await delivery.acknowledge();
      await _waitForSeq(client, activeSubscriptionId, seq);
    },
    skip: skipIfNoIntegration,
    timeout: const Timeout(Duration(seconds: 30)),
  );
}

Future<SubscriptionInfo> _subscription(
  KalamClient client,
  String subscriptionId,
) async {
  final subscriptions = await client.getSubscriptions();
  return subscriptions.singleWhere(
    (subscription) => subscription.id == subscriptionId,
    orElse: () => throw StateError(
      'Subscription $subscriptionId is not in '
      '${subscriptions.map((subscription) => subscription.id).toList()}',
    ),
  );
}

Future<void> _waitForSeq(
  KalamClient client,
  String subscriptionId,
  SeqId seq,
) async {
  final deadline = DateTime.now().add(const Duration(seconds: 5));
  while (DateTime.now().isBefore(deadline)) {
    if ((await _subscription(client, subscriptionId)).lastSeqId == seq) return;
    await Future<void>.delayed(const Duration(milliseconds: 20));
  }
  throw TimeoutException(
      'Subscription $subscriptionId did not acknowledge $seq.');
}

Future<void> _safeCancel(StreamSubscription<dynamic> subscription) async {
  try {
    await subscription.cancel().timeout(const Duration(seconds: 3));
  } on TimeoutException {
    // Client disposal performs the final native cleanup.
  }
}

Future<void> _safeDispose(KalamClient client) async {
  try {
    await client.dispose().timeout(const Duration(seconds: 3));
  } on TimeoutException {
    // The process will release any remaining native resources.
  }
}
