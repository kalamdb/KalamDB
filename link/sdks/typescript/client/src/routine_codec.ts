/**
 * HTTP metadata for `/v1/functions/{schema}/{name}`.
 *
 * Frame bytes come from the Rust domain codec in `kalam-link-wasm`
 * (`encodeRoutineRequest` / `decodeRoutineResult`).
 */

/** Typed failure from `invokeRoutine`. `code` is the server error code. */
export class KalamRoutineError extends Error {
  readonly code: string;
  readonly status: number;

  constructor(code: string, message: string, status: number) {
    super(message);
    this.name = 'KalamRoutineError';
    this.code = code;
    this.status = status;
  }
}

/** Copy frame bytes into an `ArrayBuffer` that `fetch` accepts as a body. */
export function routineRequestBody(bytes: Uint8Array): ArrayBuffer {
  const copy = new ArrayBuffer(bytes.byteLength);
  new Uint8Array(copy).set(bytes);
  return copy;
}

export function routineInvokeHeaders(input: {
  readonly epoch: number;
  readonly revision?: string;
  readonly idempotencyKey?: string;
  readonly binary: boolean;
}): Record<string, string> {
  const headers: Record<string, string> = {
    'content-type': input.binary ? 'application/x-kalam-domain' : 'application/json',
    accept: input.binary ? 'application/x-kalam-domain' : 'application/json',
    'x-kalam-epoch': String(input.epoch),
  };
  if (input.revision) headers['x-kalam-descriptor-revision'] = input.revision;
  if (input.idempotencyKey) headers['idempotency-key'] = input.idempotencyKey;
  return headers;
}

export function routineErrorFromPayload(status: number, payload: unknown): KalamRoutineError {
  const record = payload && typeof payload === 'object'
    ? payload as { code?: unknown; message?: unknown }
    : undefined;
  const code = typeof record?.code === 'string' && record.code.length > 0
    ? record.code
    : `HTTP_${status}`;
  const message = typeof record?.message === 'string' && record.message.length > 0
    ? record.message
    : `function invoke failed: ${status}`;
  return new KalamRoutineError(code, message, status);
}

/**
 * JSON text for the Rust encoder.
 *
 * Ordinary values use `JSON.stringify`. Bigints and byte arrays are marked so
 * the shared domain codec can recover them.
 */
export function routineArgumentsJson(input: unknown): string {
  return JSON.stringify(input === undefined ? null : input, (_key, value: unknown) => {
    if (typeof value === 'bigint') return { $kint: value.toString() };
    if (value instanceof Uint8Array) return { $kbytes: bytesToBase64(value) };
    return value;
  });
}

function bytesToBase64(bytes: Uint8Array): string {
  let binary = '';
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}
