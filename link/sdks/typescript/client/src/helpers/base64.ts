type RuntimeBuffer = {
  from(input: string, encoding: 'utf8' | 'base64'): Uint8Array & {
    toString(encoding: 'base64'): string;
  };
};

const BASE64_PATTERN = /^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/;
const BINARY_STRING_CHUNK_SIZE = 0x8000;

function runtimeBase64(): {
  atob?: (input: string) => string;
  btoa?: (input: string) => string;
  Buffer?: RuntimeBuffer;
} {
  return globalThis as typeof globalThis & {
    atob?: (input: string) => string;
    btoa?: (input: string) => string;
    Buffer?: RuntimeBuffer;
  };
}

export function encodeUtf8Base64(value: string): string {
  const runtime = runtimeBase64();

  if (runtime.Buffer) {
    return runtime.Buffer.from(value, 'utf8').toString('base64');
  }

  if (typeof runtime.btoa === 'function' && typeof TextEncoder === 'function') {
    const bytes = new TextEncoder().encode(value);
    let binary = '';
    for (let offset = 0; offset < bytes.length; offset += BINARY_STRING_CHUNK_SIZE) {
      binary += String.fromCharCode(...bytes.subarray(offset, offset + BINARY_STRING_CHUNK_SIZE));
    }
    return runtime.btoa(binary);
  }

  throw new Error('No base64 encoding available');
}

export function decodeBase64ToBytes(value: string): Uint8Array | null {
  const normalized = value.trim();
  if (normalized.length === 0 || normalized.length % 4 !== 0 || !BASE64_PATTERN.test(normalized)) {
    return null;
  }

  const runtime = runtimeBase64();

  try {
    if (runtime.Buffer) {
      return runtime.Buffer.from(normalized, 'base64');
    }

    if (typeof runtime.atob !== 'function') {
      return null;
    }

    const decoded = runtime.atob(normalized);
    const bytes = new Uint8Array(decoded.length);
    for (let index = 0; index < decoded.length; index += 1) {
      bytes[index] = decoded.charCodeAt(index);
    }
    return bytes;
  } catch {
    return null;
  }
}