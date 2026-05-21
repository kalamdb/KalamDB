import type { AuthCredentials, AuthProvider } from '../auth.js';

export interface AuthProviderRetryOptions {
  maxAttempts?: number;
  initialBackoffMs?: number;
  maxBackoffMs?: number;
  shouldRetry?: (error: unknown, attempt: number) => boolean;
  sleep?: (ms: number) => Promise<void>;
}

const DEFAULT_RETRY: Required<Omit<AuthProviderRetryOptions, 'shouldRetry' | 'sleep'>> = {
  maxAttempts: 3,
  initialBackoffMs: 250,
  maxBackoffMs: 2000,
};

const TRANSIENT_AUTH_PROVIDER_MARKERS = [
  'timeout',
  'timed out',
  'network',
  'socket',
  'connection',
  'unreachable',
  'temporar',
  '429',
  '502',
  '503',
  '504',
];

function errorStatus(error: unknown): number | undefined {
  if (!error || typeof error !== 'object') {
    return undefined;
  }

  const status = (error as { status?: unknown; statusCode?: unknown }).status
    ?? (error as { statusCode?: unknown }).statusCode;
  return typeof status === 'number' && Number.isInteger(status) ? status : undefined;
}

function defaultSleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

export function isLikelyTransientAuthProviderError(error: unknown): boolean {
  const status = errorStatus(error);
  if (status === 429 || status === 502 || status === 503 || status === 504) {
    return true;
  }

  if (error instanceof Error && /timeout|network|abort/i.test(error.name)) {
    return true;
  }

  const message = String(error).toLowerCase();
  return TRANSIENT_AUTH_PROVIDER_MARKERS.some((marker) => message.includes(marker));
}

export async function resolveAuthProviderWithRetry(
  provider: AuthProvider,
  options: AuthProviderRetryOptions = {},
): Promise<AuthCredentials> {
  const maxAttempts = Math.max(1, Math.floor(options.maxAttempts ?? DEFAULT_RETRY.maxAttempts));
  const initialBackoffMs = Math.max(0, Math.floor(options.initialBackoffMs ?? DEFAULT_RETRY.initialBackoffMs));
  const maxBackoffMs = Math.max(initialBackoffMs, Math.floor(options.maxBackoffMs ?? DEFAULT_RETRY.maxBackoffMs));
  const shouldRetry = options.shouldRetry ?? ((error: unknown) => isLikelyTransientAuthProviderError(error));
  const sleep = options.sleep ?? defaultSleep;

  let backoffMs = initialBackoffMs;
  for (let attempt = 1; attempt <= maxAttempts; attempt += 1) {
    try {
      return await provider();
    } catch (error) {
      const hasMore = attempt < maxAttempts;
      if (!hasMore || !shouldRetry(error, attempt)) {
        throw error;
      }
      if (backoffMs > 0) {
        await sleep(backoffMs);
      }
      backoffMs = Math.min(maxBackoffMs, backoffMs * 2);
    }
  }

  throw new Error('Unreachable: auth provider retry loop exited');
}
