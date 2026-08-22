import 'package:flutter_test/flutter_test.dart';
import 'package:kalam_sync/kalam_sync.dart';

void main() {
  test('decodes initial rows and removes the internal sequence column', () {
    final changes = KalamLinkTransport.decodeEvent(
      InitialDataBatch(
        subscriptionId: 'messages',
        rowsJson: const ['{"id":"message-1","text":"hello","_seq":12}'],
        batchNum: 1,
        hasMore: false,
        status: 'ok',
      ),
    );

    expect(changes.single.initial, isTrue);
    expect(changes.single.seq, const SeqId(12));
    expect(changes.single.row, {'id': 'message-1', 'text': 'hello'});
  });

  test('sorts batched changes by sequence before local application', () {
    final changes = KalamLinkTransport.decodeEvent(
      InsertEvent(
        subscriptionId: 'messages',
        rowsJson: const [
          '{"id":"message-2","_seq":2}',
          '{"id":"message-1","_seq":1}',
        ],
      ),
    );

    expect(changes.map((change) => change.seq), [
      const SeqId(1),
      const SeqId(2),
    ]);
  });

  test('keeps old update values available for reconciliation', () {
    final change = KalamLinkTransport.decodeEvent(
      UpdateEvent(
        subscriptionId: 'messages',
        rowsJson: const ['{"id":"message-1","text":"new","_seq":2}'],
        oldRowsJson: const ['{"id":"message-1","text":"old","_seq":1}'],
      ),
    ).single;

    expect(change.kind, KalamChangeKind.update);
    expect(change.oldRow, {'id': 'message-1', 'text': 'old'});
  });

  test('surfaces subscription errors', () {
    expect(
      () => KalamLinkTransport.decodeEvent(
        const SubscriptionError(
          subscriptionId: 'messages',
          code: 'CURSOR_EXPIRED',
          message: 'checkpoint is outside retention',
        ),
      ),
      throwsA(isA<KalamSubscriptionException>()),
    );
  });

  test('rejects synchronized rows without a sequence', () {
    expect(
      () => KalamLinkTransport.decodeEvent(
        InsertEvent(
          subscriptionId: 'messages',
          rowsJson: const ['{"id":"message-1"}'],
        ),
      ),
      throwsFormatException,
    );
  });
}
