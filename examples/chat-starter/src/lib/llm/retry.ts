// Wraps an LLM adapter's stream() so that the *initial* request (the fetch
// before any tokens have been observed) is retried on transient errors with
// exponential backoff. Once the stream has started yielding events, retries
// would replay tokens — caller is expected to handle that themselves.
//
// Retryable conditions:
//   - Network errors (TypeError thrown by fetch)
//   - HTTP 429 or 5xx (we surface via Error message text from the adapter)
//
// Not retried:
//   - AbortError / aborted signal
//   - HTTP 4xx (other than 429) — caller's fault, won't fix on retry

import { LlmHttpError, type LlmAdapter, type LlmStreamArgs, type LlmStreamEvent } from "./index.js";
import type { Logger } from "../logger.js";

export interface RetryPolicy {
  maxAttempts: number;
  baseDelayMs: number;
  maxDelayMs: number;
}

export const DEFAULT_RETRY_POLICY: RetryPolicy = {
  maxAttempts: 3,
  baseDelayMs: 500,
  maxDelayMs: 5_000,
};

function isAbort(err: unknown, signal: AbortSignal): boolean {
  if (signal.aborted) return true;
  if (err instanceof Error) {
    if (err.name === "AbortError") return true;
    if (err.message.toLowerCase().includes("aborted")) return true;
  }
  return false;
}

function isRetryable(err: unknown): boolean {
  if (err instanceof TypeError) return true; // fetch network failure
  if (err instanceof LlmHttpError) {
    return err.status === 429 || (err.status >= 500 && err.status < 600);
  }
  return false;
}

function backoff(attempt: number, policy: RetryPolicy): number {
  const base = Math.min(policy.maxDelayMs, policy.baseDelayMs * 2 ** (attempt - 1));
  // Full jitter — Amazon-style.
  return Math.random() * base;
}

function sleep(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal.aborted) return reject(new Error("aborted"));
    const t = setTimeout(() => {
      signal.removeEventListener("abort", onAbort);
      resolve();
    }, ms);
    const onAbort = (): void => {
      clearTimeout(t);
      reject(new Error("aborted"));
    };
    signal.addEventListener("abort", onAbort, { once: true });
  });
}

export function withRetry(
  inner: LlmAdapter,
  policy: RetryPolicy = DEFAULT_RETRY_POLICY,
  log?: Logger,
): LlmAdapter {
  return {
    name: inner.name,
    async *stream(args: LlmStreamArgs): AsyncGenerator<LlmStreamEvent, void, undefined> {
      let attempt = 0;
      // Outer loop: retry the *initial* connect+first event.
      while (true) {
        attempt += 1;
        let started = false;
        try {
          for await (const event of inner.stream(args)) {
            started = true;
            yield event;
          }
          return;
        } catch (err) {
          if (started) throw err;
          if (isAbort(err, args.signal)) throw err;
          if (attempt >= policy.maxAttempts || !isRetryable(err)) throw err;
          const delay = backoff(attempt, policy);
          log?.warn(
            { attempt, max: policy.maxAttempts, delay_ms: Math.round(delay), err },
            "llm transient error; retrying",
          );
          await sleep(delay, args.signal);
        }
      }
    },
  };
}
