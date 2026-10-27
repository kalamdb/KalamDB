import 'package:flutter_test/flutter_test.dart';
import 'package:kalam_link/src/invoke_routine_io.dart';

void main() {
  test('sends the descriptor revision header only when a revision is present', () {
    final headers = routineInvokeHeaders(
      epoch: 1,
      descriptorRevision: 'abc123',
      binary: false,
    );
    expect(headers['x-kalam-epoch'], '1');
    expect(headers['x-kalam-descriptor-revision'], 'abc123');
    expect(headers['content-type'], 'application/json');

    final omitted = routineInvokeHeaders(epoch: 1, binary: true);
    expect(omitted.containsKey('x-kalam-descriptor-revision'), isFalse);
    expect(omitted['content-type'], 'application/x-kalam-domain');
  });

  test('preserves the server error code', () {
    final error = routineExceptionFromPayload(409, {
      'code': 'STALE_REVISION',
      'message': 'descriptor revision mismatch',
    });
    expect(error, isA<KalamRoutineException>());
    expect(error.code, 'STALE_REVISION');
    expect(error.statusCode, 409);
    expect(error.message, 'descriptor revision mismatch');
  });
}
