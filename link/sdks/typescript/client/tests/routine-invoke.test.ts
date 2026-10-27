import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import {
  KalamRoutineError,
  routineArgumentsJson,
  routineErrorFromPayload,
  routineInvokeHeaders,
} from '../src/routine_codec.ts';

describe('invokeRoutine contract', () => {
  it('sends the descriptor revision header only when a revision is present', () => {
    const headers = routineInvokeHeaders({
      epoch: 1,
      revision: 'abc123',
      binary: false,
    });
    assert.equal(headers['x-kalam-epoch'], '1');
    assert.equal(headers['x-kalam-descriptor-revision'], 'abc123');
    assert.equal(headers['content-type'], 'application/json');

    const omitted = routineInvokeHeaders({ epoch: 1, binary: true });
    assert.equal(omitted['x-kalam-descriptor-revision'], undefined);
    assert.equal(omitted['content-type'], 'application/x-kalam-domain');
  });

  it('preserves the server error code', () => {
    const error = routineErrorFromPayload(409, {
      code: 'STALE_REVISION',
      message: 'descriptor revision mismatch',
    });
    assert.ok(error instanceof KalamRoutineError);
    assert.equal(error.code, 'STALE_REVISION');
    assert.equal(error.status, 409);
    assert.equal(error.message, 'descriptor revision mismatch');
  });

  it('keeps bigint arguments as JSON integers for the Rust encoder', () => {
    assert.equal(routineArgumentsJson({ n: 5n, missing: undefined }), '{"n":{"$kint":"5"}}');
  });
});
